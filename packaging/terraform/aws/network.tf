resource "aws_vpc" "this" {
  count                = local.create_vpc ? 1 : 0
  cidr_block           = var.vpc_cidr
  enable_dns_hostnames = true
  enable_dns_support   = true

  lifecycle {
    precondition {
      condition     = length(var.azs) >= 2
      error_message = "Provide at least 2 availability zones in `azs`, or set `vpc_id` + `public_subnet_ids` + `private_subnet_ids` to deploy into an existing VPC."
    }
  }

  tags = merge(local.tags, { Name = local.name })
}

resource "aws_internet_gateway" "this" {
  count  = local.create_vpc ? 1 : 0
  vpc_id = aws_vpc.this[0].id

  tags = merge(local.tags, { Name = local.name })
}

resource "aws_subnet" "public" {
  count                   = local.create_vpc ? length(var.azs) : 0
  vpc_id                  = aws_vpc.this[0].id
  cidr_block              = cidrsubnet(var.vpc_cidr, 8, count.index)
  availability_zone       = var.azs[count.index]
  map_public_ip_on_launch = true

  tags = merge(local.tags, { Name = "${local.name}-public-${var.azs[count.index]}" })
}

resource "aws_subnet" "private" {
  count             = local.create_vpc ? length(var.azs) : 0
  vpc_id            = aws_vpc.this[0].id
  cidr_block        = cidrsubnet(var.vpc_cidr, 8, count.index + 10)
  availability_zone = var.azs[count.index]

  tags = merge(local.tags, { Name = "${local.name}-private-${var.azs[count.index]}" })
}

resource "aws_eip" "nat" {
  count  = local.create_vpc ? 1 : 0
  domain = "vpc"

  depends_on = [aws_internet_gateway.this]

  tags = merge(local.tags, { Name = "${local.name}-nat" })
}

# One NAT gateway in the first public subnet; a gateway per AZ costs more and is left to the caller
resource "aws_nat_gateway" "this" {
  count         = local.create_vpc ? 1 : 0
  allocation_id = aws_eip.nat[0].id
  subnet_id     = aws_subnet.public[0].id

  depends_on = [aws_internet_gateway.this]

  tags = merge(local.tags, { Name = local.name })
}

resource "aws_route_table" "public" {
  count  = local.create_vpc ? 1 : 0
  vpc_id = aws_vpc.this[0].id

  route {
    cidr_block = "0.0.0.0/0"
    gateway_id = aws_internet_gateway.this[0].id
  }

  tags = merge(local.tags, { Name = "${local.name}-public" })
}

resource "aws_route_table_association" "public" {
  count          = local.create_vpc ? length(var.azs) : 0
  subnet_id      = aws_subnet.public[count.index].id
  route_table_id = aws_route_table.public[0].id
}

resource "aws_route_table" "private" {
  count  = local.create_vpc ? 1 : 0
  vpc_id = aws_vpc.this[0].id

  route {
    cidr_block     = "0.0.0.0/0"
    nat_gateway_id = aws_nat_gateway.this[0].id
  }

  tags = merge(local.tags, { Name = "${local.name}-private" })
}

resource "aws_route_table_association" "private" {
  count          = local.create_vpc ? length(var.azs) : 0
  subnet_id      = aws_subnet.private[count.index].id
  route_table_id = aws_route_table.private[0].id
}

resource "aws_security_group" "alb" {
  name        = "${local.name}-alb"
  description = "Inbound HTTP and HTTPS to the load balancer."
  vpc_id      = local.vpc_id

  ingress {
    description = "HTTP from anywhere"
    from_port   = 80
    to_port     = 80
    protocol    = "tcp"
    cidr_blocks = ["0.0.0.0/0"]
  }

  ingress {
    description = "HTTPS from anywhere"
    from_port   = 443
    to_port     = 443
    protocol    = "tcp"
    cidr_blocks = ["0.0.0.0/0"]
  }

  egress {
    description = "All egress"
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
  }

  tags = local.tags
}

# This group exists in every mode, so the supplied-subnet check lives here rather than on a resource that can go to zero
resource "aws_security_group" "tasks" {
  name        = "${local.name}-tasks"
  description = "ECS tasks: gateway, backend and UI."
  vpc_id      = local.vpc_id

  ingress {
    description     = "Load balancer to tasks"
    from_port       = 0
    to_port         = 65535
    protocol        = "tcp"
    security_groups = [aws_security_group.alb.id]
  }

  egress {
    description = "All egress (LLM providers, RDS, Redis)"
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
  }

  lifecycle {
    precondition {
      condition     = local.create_vpc || length(var.private_subnet_ids) >= (local.managed_stores_need_two_azs ? 2 : 1)
      error_message = "`private_subnet_ids` is required when `vpc_id` is set: the tasks, Aurora and ElastiCache all live in private subnets. Aurora and ElastiCache subnet groups need subnets in at least 2 AZs, so pass 2 unless both `create_database` and `create_redis` are false."
    }
  }

  tags = local.tags
}

resource "aws_security_group" "rds" {
  count       = var.create_database ? 1 : 0
  name        = "${local.name}-rds"
  description = "Aurora Postgres, reachable from the tasks only."
  vpc_id      = local.vpc_id

  ingress {
    description     = "Postgres from ECS tasks"
    from_port       = 5432
    to_port         = 5432
    protocol        = "tcp"
    security_groups = [aws_security_group.tasks.id]
  }

  egress {
    description = "All egress"
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
  }

  tags = local.tags
}

resource "aws_security_group" "redis" {
  count       = var.create_redis ? 1 : 0
  name        = "${local.name}-redis"
  description = "ElastiCache Redis, reachable from the tasks only."
  vpc_id      = local.vpc_id

  ingress {
    description     = "Redis from ECS tasks"
    from_port       = 6379
    to_port         = 6379
    protocol        = "tcp"
    security_groups = [aws_security_group.tasks.id]
  }

  egress {
    description = "All egress"
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
  }

  tags = local.tags
}
