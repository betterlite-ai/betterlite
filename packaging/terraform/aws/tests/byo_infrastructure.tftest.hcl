mock_provider "aws" {
  mock_data "aws_iam_policy_document" {
    defaults = {
      json = "{\"Version\":\"2012-10-17\",\"Statement\":[]}"
    }
  }
}
mock_provider "random" {}

variables {
  region = "us-east-1"
  tenant = "acme"
  env    = "test"
}

run "module_owns_everything_by_default" {
  command = plan

  variables {
    azs = ["us-east-1a", "us-east-1b"]
  }

  assert {
    condition     = length(aws_vpc.this) == 1 && length(aws_nat_gateway.this) == 1 && length(aws_subnet.private) == 2
    error_message = "The default path must still create its own VPC, NAT gateway, and one private subnet per AZ."
  }

  assert {
    condition     = length(aws_subnet.public) == 2 && length(aws_route_table.public) == 1 && length(aws_route_table.private) == 1
    error_message = "The default path must create one public subnet per AZ and one route table for each subnet kind."
  }

  assert {
    condition     = length(aws_rds_cluster.this) == 1 && length(aws_elasticache_replication_group.this) == 1
    error_message = "The default path must still create Aurora and ElastiCache."
  }

  assert {
    condition     = length(aws_rds_cluster_instance.writer) == 1 && length(aws_rds_cluster_instance.reader) == 1
    error_message = "Aurora must come with one writer and one reader instance."
  }

  assert {
    condition     = length(aws_security_group.rds) == 1 && length(aws_security_group.redis) == 1
    error_message = "The module-created stores each need a security group that admits the tasks."
  }
}

run "existing_vpc_creates_no_networking" {
  command = plan

  variables {
    vpc_id                             = "vpc-00000000000000001"
    public_subnet_ids                  = ["subnet-pub-a", "subnet-pub-b"]
    private_subnet_ids                 = ["subnet-priv-a", "subnet-priv-b"]
    additional_task_security_group_ids = ["sg-caller-owned"]
  }

  assert {
    condition = alltrue([
      length(aws_vpc.this) == 0,
      length(aws_subnet.public) == 0,
      length(aws_subnet.private) == 0,
      length(aws_internet_gateway.this) == 0,
      length(aws_nat_gateway.this) == 0,
      length(aws_eip.nat) == 0,
      length(aws_route_table.public) == 0,
      length(aws_route_table.private) == 0,
    ])
    error_message = "An existing vpc_id must suppress every network resource, including the route tables and NAT gateway."
  }

  assert {
    condition = alltrue([
      aws_db_subnet_group.this[0].subnet_ids == toset(var.private_subnet_ids),
      aws_elasticache_subnet_group.this[0].subnet_ids == toset(var.private_subnet_ids),
    ])
    error_message = "Aurora and ElastiCache must land in the caller's private subnets."
  }

  assert {
    condition     = length(local.task_security_group_ids) == 2
    error_message = "additional_task_security_group_ids must be attached alongside the module's own tasks group."
  }
}

run "existing_database_and_redis_replace_the_managed_ones" {
  command = plan

  variables {
    azs             = ["us-east-1a", "us-east-1b"]
    create_database = false
    database_url    = "postgresql://litellm:pw@db.internal:5432/litellm"
    create_redis    = false
    redis_url       = "rediss://:pw@cache.internal:6379"
  }

  assert {
    condition = alltrue([
      length(aws_rds_cluster.this) == 0,
      length(aws_rds_cluster_instance.writer) == 0,
      length(aws_rds_cluster_instance.reader) == 0,
      length(aws_db_subnet_group.this) == 0,
      length(aws_rds_cluster_parameter_group.this) == 0,
      length(random_password.db_master_password) == 0,
      length(aws_security_group.rds) == 0,
      length(aws_elasticache_replication_group.this) == 0,
      length(aws_elasticache_subnet_group.this) == 0,
      length(aws_security_group.redis) == 0,
    ])
    error_message = "Pointing at an existing database and cache must create neither Aurora nor ElastiCache."
  }

  assert {
    condition     = local.byo_database && local.database_enabled && local.byo_redis && local.redis_enabled
    error_message = "A supplied database_url and redis_url must keep both stores enabled through the bring-your-own flags."
  }
}

run "vpc_without_subnets_fails_at_plan" {
  command = plan

  variables {
    vpc_id = "vpc-00000000000000001"
  }

  expect_failures = [
    aws_security_group.tasks,
  ]
}

run "neither_vpc_nor_azs_fails_at_plan" {
  command = plan

  expect_failures = [
    aws_vpc.this,
  ]
}

run "one_private_subnet_fails_while_a_managed_store_needs_two_azs" {
  command = plan

  variables {
    vpc_id             = "vpc-00000000000000001"
    public_subnet_ids  = ["subnet-pub-a", "subnet-pub-b"]
    private_subnet_ids = ["subnet-priv-a"]
  }

  expect_failures = [
    aws_security_group.tasks,
  ]
}

run "one_private_subnet_is_enough_without_managed_stores" {
  command = plan

  variables {
    vpc_id             = "vpc-00000000000000001"
    public_subnet_ids  = ["subnet-pub-a", "subnet-pub-b"]
    private_subnet_ids = ["subnet-priv-a"]
    create_database    = false
    create_redis       = false
  }

  assert {
    condition     = !local.database_enabled && !local.redis_enabled && length(aws_security_group.tasks.name) > 0
    error_message = "With no module-created database or cache, a single private subnet must plan cleanly."
  }
}
