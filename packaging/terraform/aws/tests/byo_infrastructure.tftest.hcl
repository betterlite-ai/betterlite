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
      tolist(local.public_subnet_ids) == var.public_subnet_ids,
      tolist(local.private_subnet_ids) == var.private_subnet_ids,
      aws_db_subnet_group.this[0].subnet_ids == toset(var.private_subnet_ids),
      aws_elasticache_subnet_group.this[0].subnet_ids == toset(var.private_subnet_ids),
    ])
    error_message = "Aurora and ElastiCache must land in the caller's private subnets."
  }

  assert {
    condition = alltrue([
      aws_security_group.alb.vpc_id == var.vpc_id,
      aws_security_group.tasks.vpc_id == var.vpc_id,
      aws_security_group.rds[0].vpc_id == var.vpc_id,
      aws_security_group.redis[0].vpc_id == var.vpc_id,
    ])
    error_message = "The security groups are module-owned even in a supplied VPC, so they must attach to it."
  }

  assert {
    condition = alltrue([
      length(local.tags) == 2,
      local.tags["litellm:stack"] == local.name,
      local.tags["managed-by"] == "terraform",
    ])
    error_message = "With no var.tags the module's own two tags are the whole tag set."
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

run "skip_final_snapshot_drops_the_final_snapshot_name" {
  command = plan

  variables {
    azs                 = ["us-east-1a", "us-east-1b"]
    skip_final_snapshot = true
  }

  assert {
    condition = alltrue([
      aws_rds_cluster.this[0].skip_final_snapshot == true,
      aws_rds_cluster.this[0].final_snapshot_identifier == null,
    ])
    error_message = "Skipping the final snapshot must drop its name, since setting both is an apply-time error."
  }
}

run "redis_replicas_zero_turns_off_failover" {
  command = plan

  variables {
    azs                = ["us-east-1a", "us-east-1b"]
    redis_num_replicas = 0
  }

  assert {
    condition = alltrue([
      aws_elasticache_replication_group.this[0].num_cache_clusters == 1,
      aws_elasticache_replication_group.this[0].automatic_failover_enabled == false,
      aws_elasticache_replication_group.this[0].multi_az_enabled == false,
    ])
    error_message = "Without replicas the group is one node, which can have neither automatic failover nor multi-AZ."
  }
}

run "redis_replicas_add_nodes_and_turn_on_failover" {
  command = plan

  variables {
    azs                = ["us-east-1a", "us-east-1b"]
    redis_num_replicas = 2
  }

  assert {
    condition = alltrue([
      aws_elasticache_replication_group.this[0].num_cache_clusters == 3,
      aws_elasticache_replication_group.this[0].automatic_failover_enabled == true,
      aws_elasticache_replication_group.this[0].multi_az_enabled == true,
    ])
    error_message = "Replicas are added on top of the primary and switch on automatic failover and multi-AZ."
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

run "one_az_fails_at_plan" {
  command = plan

  variables {
    azs = ["us-east-1a"]
  }

  expect_failures = [
    aws_vpc.this,
  ]
}

run "uppercase_env_fails_its_validation" {
  command = plan

  variables {
    azs = ["us-east-1a", "us-east-1b"]
    env = "Test"
  }

  expect_failures = [
    var.env,
  ]
}

run "overlong_env_fails_its_validation" {
  command = plan

  variables {
    azs = ["us-east-1a", "us-east-1b"]
    env = "abcdefghij"
  }

  expect_failures = [
    var.env,
  ]
}

run "uppercase_tenant_fails_its_validation" {
  command = plan

  variables {
    azs    = ["us-east-1a", "us-east-1b"]
    tenant = "Acme"
  }

  expect_failures = [
    var.tenant,
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

run "no_private_subnet_fails_even_without_managed_stores" {
  command = plan

  variables {
    vpc_id             = "vpc-00000000000000001"
    public_subnet_ids  = ["subnet-pub-a", "subnet-pub-b"]
    private_subnet_ids = []
    create_database    = false
    create_redis       = false
  }

  expect_failures = [
    aws_security_group.tasks,
  ]
}

run "negative_redis_replicas_fail_their_validation" {
  command = plan

  variables {
    azs                = ["us-east-1a", "us-east-1b"]
    redis_num_replicas = -1
  }

  expect_failures = [
    var.redis_num_replicas,
  ]
}

run "module_owns_everything_by_default" {
  # apply, not plan: the asserts compare resource ids, which a plan leaves unknown. The mocked providers make it offline.
  command = apply

  variables {
    azs  = ["us-east-1a", "us-east-1b"]
    tags = { owner = "platform" }
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
    condition = alltrue([
      aws_vpc.this[0].enable_dns_hostnames == true,
      aws_vpc.this[0].enable_dns_support == true,
      aws_internet_gateway.this[0].vpc_id == aws_vpc.this[0].id,
    ])
    error_message = "The VPC must keep DNS support and hostnames on, with the internet gateway attached to it."
  }

  assert {
    condition = alltrue([
      aws_subnet.public[0].cidr_block == cidrsubnet(var.vpc_cidr, 8, 0),
      aws_subnet.public[1].cidr_block == cidrsubnet(var.vpc_cidr, 8, 1),
      aws_subnet.private[0].cidr_block == cidrsubnet(var.vpc_cidr, 8, 10),
      aws_subnet.private[1].cidr_block == cidrsubnet(var.vpc_cidr, 8, 11),
      aws_subnet.public[1].availability_zone == var.azs[1],
      aws_subnet.private[0].availability_zone == var.azs[0],
      aws_subnet.public[0].map_public_ip_on_launch == true,
    ])
    error_message = "Each subnet must take the AZ at its index, public subnets cidrsubnet(vpc_cidr, 8, index) with public IPs on launch and private ones the same index offset by ten."
  }

  assert {
    condition = alltrue([
      one(aws_route_table.public[0].route).gateway_id == aws_internet_gateway.this[0].id,
      one(aws_route_table.private[0].route).nat_gateway_id == aws_nat_gateway.this[0].id,
      aws_nat_gateway.this[0].subnet_id == aws_subnet.public[0].id,
      aws_nat_gateway.this[0].allocation_id == aws_eip.nat[0].id,
      aws_eip.nat[0].domain == "vpc",
    ])
    error_message = "The public route must go through the internet gateway and the private route through the single NAT gateway in the first public subnet."
  }

  assert {
    condition = alltrue([
      length(aws_route_table_association.public) == length(var.azs),
      length(aws_route_table_association.private) == length(var.azs),
      alltrue([for association in aws_route_table_association.public : association.route_table_id == aws_route_table.public[0].id]),
      alltrue([for association in aws_route_table_association.private : association.route_table_id == aws_route_table.private[0].id]),
      alltrue([for association in aws_route_table_association.public : contains(tolist(aws_subnet.public[*].id), association.subnet_id)]),
      alltrue([for association in aws_route_table_association.private : contains(tolist(aws_subnet.private[*].id), association.subnet_id)]),
    ])
    error_message = "Every public and private subnet needs one association, and each association must join its subnet to the matching route table."
  }

  assert {
    condition = alltrue([
      aws_db_subnet_group.this[0].subnet_ids == toset(local.private_subnet_ids),
      aws_elasticache_subnet_group.this[0].subnet_ids == toset(local.private_subnet_ids),
      tolist(local.public_subnet_ids) == tolist(aws_subnet.public[*].id),
      tolist(local.private_subnet_ids) == tolist(aws_subnet.private[*].id),
    ])
    error_message = "Both store subnet groups must take the module's private subnets, and the subnet locals must read the subnets they name."
  }

  assert {
    condition = alltrue([
      aws_security_group.alb.name == "${local.name}-alb",
      aws_security_group.tasks.name == "${local.name}-tasks",
      aws_security_group.rds[0].name == "${local.name}-rds",
      aws_security_group.redis[0].name == "${local.name}-redis",
      aws_db_subnet_group.this[0].name == "${local.name}-db",
      aws_rds_cluster_parameter_group.this[0].name == "${local.name}-cluster-pg",
      aws_elasticache_subnet_group.this[0].name == "${local.name}-redis",
    ])
    error_message = "Every resource must keep the <tenant>-litellm-<env> naming with the per-resource suffix LiteLLM's module uses."
  }

  assert {
    condition = alltrue([
      aws_security_group.alb.vpc_id == aws_vpc.this[0].id,
      aws_security_group.tasks.vpc_id == aws_vpc.this[0].id,
      aws_security_group.rds[0].vpc_id == aws_vpc.this[0].id,
      aws_security_group.redis[0].vpc_id == aws_vpc.this[0].id,
      length(local.task_security_group_ids) == 1,
    ])
    error_message = "All four security groups must live in the VPC the module creates, and with no extra groups the tasks carry only the module's own."
  }

  assert {
    condition = alltrue([
      length(aws_security_group.alb.ingress) == 2,
      length([for rule in aws_security_group.alb.ingress : rule if rule.from_port == 80 && rule.to_port == 80 && contains(rule.cidr_blocks, "0.0.0.0/0")]) == 1,
      length([for rule in aws_security_group.alb.ingress : rule if rule.from_port == 443 && rule.to_port == 443 && contains(rule.cidr_blocks, "0.0.0.0/0")]) == 1,
      length(aws_security_group.tasks.ingress) == 1,
      one(aws_security_group.tasks.ingress).from_port == 0,
      one(aws_security_group.tasks.ingress).to_port == 65535,
      one(aws_security_group.tasks.ingress).protocol == "tcp",
      contains(one(aws_security_group.tasks.ingress).security_groups, aws_security_group.alb.id),
      try(length(one(aws_security_group.tasks.ingress).cidr_blocks), 0) == 0,
    ])
    error_message = "The ALB must admit HTTP and HTTPS from anywhere, and the tasks must admit every port from the ALB group only."
  }

  assert {
    condition = alltrue([
      length(aws_security_group.rds[0].ingress) == 1,
      one(aws_security_group.rds[0].ingress).from_port == 5432,
      one(aws_security_group.rds[0].ingress).to_port == 5432,
      one(aws_security_group.rds[0].ingress).protocol == "tcp",
      contains(one(aws_security_group.rds[0].ingress).security_groups, aws_security_group.tasks.id),
      try(length(one(aws_security_group.rds[0].ingress).cidr_blocks), 0) == 0,
      length(aws_security_group.redis[0].ingress) == 1,
      one(aws_security_group.redis[0].ingress).from_port == 6379,
      one(aws_security_group.redis[0].ingress).to_port == 6379,
      contains(one(aws_security_group.redis[0].ingress).security_groups, aws_security_group.tasks.id),
      try(length(one(aws_security_group.redis[0].ingress).cidr_blocks), 0) == 0,
    ])
    error_message = "Aurora must admit 5432 and ElastiCache 6379 from the tasks group, never from a CIDR range."
  }

  # The rds and redis egress rules are the deviation recorded in docs/deviations/B8140.yaml
  assert {
    condition = alltrue([
      length(aws_security_group.alb.egress) == 1,
      alltrue([for rule in aws_security_group.alb.egress : rule.protocol == "-1" && rule.from_port == 0 && rule.to_port == 0 && contains(rule.cidr_blocks, "0.0.0.0/0")]),
      length(aws_security_group.tasks.egress) == 1,
      alltrue([for rule in aws_security_group.tasks.egress : rule.protocol == "-1" && rule.from_port == 0 && rule.to_port == 0 && contains(rule.cidr_blocks, "0.0.0.0/0")]),
      length(aws_security_group.rds[0].egress) == 1,
      alltrue([for rule in aws_security_group.rds[0].egress : rule.protocol == "-1" && rule.from_port == 0 && rule.to_port == 0 && contains(rule.cidr_blocks, "0.0.0.0/0")]),
      length(aws_security_group.redis[0].egress) == 1,
      alltrue([for rule in aws_security_group.redis[0].egress : rule.protocol == "-1" && rule.from_port == 0 && rule.to_port == 0 && contains(rule.cidr_blocks, "0.0.0.0/0")]),
    ])
    error_message = "Every module-owned group allows all egress, as the module's deviation log records."
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
    condition = alltrue([
      aws_rds_cluster.this[0].engine == "aurora-postgresql",
      aws_rds_cluster.this[0].engine_mode == "provisioned",
      aws_rds_cluster.this[0].engine_version == var.db_engine_version,
      aws_rds_cluster.this[0].cluster_identifier == "${var.tenant}-litellm-${var.env}",
      aws_rds_cluster.this[0].database_name == "litellm",
      aws_rds_cluster.this[0].master_username == "postgres",
      aws_rds_cluster.this[0].iam_database_authentication_enabled == true,
      aws_rds_cluster.this[0].storage_encrypted == true,
      aws_rds_cluster.this[0].apply_immediately == true,
      aws_rds_cluster.this[0].db_subnet_group_name == aws_db_subnet_group.this[0].name,
      contains(aws_rds_cluster.this[0].vpc_security_group_ids, aws_security_group.rds[0].id),
      aws_rds_cluster.this[0].db_cluster_parameter_group_name == aws_rds_cluster_parameter_group.this[0].name,
      aws_rds_cluster_parameter_group.this[0].family == "aurora-postgresql16",
      aws_rds_cluster.this[0].backup_retention_period == 7,
      aws_rds_cluster.this[0].preferred_backup_window == "07:00-09:00",
      aws_rds_cluster.this[0].skip_final_snapshot == false,
      aws_rds_cluster.this[0].final_snapshot_identifier == "${local.name}-final-${substr(md5(local.name), 0, 8)}",
    ])
    error_message = "Aurora must stay a provisioned Postgres cluster with IAM auth, encryption, the LiteLLM backup window and a final snapshot named from the stack."
  }

  assert {
    condition = alltrue([
      length(random_password.db_master_password) == 1,
      random_password.db_master_password[0].length == 32,
      random_password.db_master_password[0].special == false,
      random_password.db_master_password[0].min_lower == 4,
      random_password.db_master_password[0].min_upper == 4,
      random_password.db_master_password[0].min_numeric == 4,
    ])
    error_message = "The Aurora master password must be 32 alphanumeric characters with at least four of each case and four digits."
  }

  assert {
    condition = alltrue([
      aws_rds_cluster_instance.writer[0].instance_class == var.db_instance_class,
      aws_rds_cluster_instance.writer[0].cluster_identifier == aws_rds_cluster.this[0].id,
      aws_rds_cluster_instance.writer[0].promotion_tier == 0,
      aws_rds_cluster_instance.writer[0].publicly_accessible == false,
      aws_rds_cluster_instance.writer[0].performance_insights_enabled == true,
      aws_rds_cluster_instance.reader[0].instance_class == "db.r6g.large",
      aws_rds_cluster_instance.reader[0].cluster_identifier == aws_rds_cluster.this[0].id,
      aws_rds_cluster_instance.reader[0].promotion_tier == 15,
      aws_rds_cluster_instance.reader[0].publicly_accessible == false,
      aws_rds_cluster_instance.reader[0].performance_insights_enabled == true,
    ])
    error_message = "The writer stays first in the failover order and the reader last, neither is publicly accessible, and both keep performance insights on."
  }

  assert {
    condition = alltrue([
      aws_elasticache_replication_group.this[0].engine == "redis",
      aws_elasticache_replication_group.this[0].engine_version == "7.1",
      aws_elasticache_replication_group.this[0].port == 6379,
      aws_elasticache_replication_group.this[0].parameter_group_name == "default.redis7",
      aws_elasticache_replication_group.this[0].replication_group_id == "${var.tenant}-litellm-${var.env}-redis",
      aws_elasticache_replication_group.this[0].node_type == var.redis_node_type,
      aws_elasticache_replication_group.this[0].subnet_group_name == aws_elasticache_subnet_group.this[0].name,
      contains(aws_elasticache_replication_group.this[0].security_group_ids, aws_security_group.redis[0].id),
      var.redis_num_replicas == 1,
      aws_elasticache_replication_group.this[0].num_cache_clusters == 1 + var.redis_num_replicas,
      aws_elasticache_replication_group.this[0].automatic_failover_enabled == (var.redis_num_replicas >= 1),
      aws_elasticache_replication_group.this[0].multi_az_enabled == (var.redis_num_replicas >= 1),
      aws_elasticache_replication_group.this[0].apply_immediately == true,
      tostring(aws_elasticache_replication_group.this[0].at_rest_encryption_enabled) == "true",
      aws_elasticache_replication_group.this[0].transit_encryption_enabled == true,
    ])
    error_message = "Redis must be a TLS-only 7.1 replication group of a primary plus the default replica, with automatic failover and multi-AZ on."
  }

  assert {
    condition = alltrue([
      aws_rds_cluster.this[0].tags["litellm:stack"] == local.name,
      aws_rds_cluster.this[0].tags["managed-by"] == "terraform",
      aws_rds_cluster.this[0].tags["owner"] == "platform",
      length(aws_rds_cluster.this[0].tags) == 3,
    ])
    error_message = "Every resource must carry the stack and managed-by tags, with var.tags layered on top of them."
  }
}
