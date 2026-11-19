mock_provider "aws" {
  mock_data "aws_iam_policy_document" {
    defaults = {
      json = "{\"Version\":\"2012-10-17\",\"Statement\":[]}"
    }
  }

  mock_data "aws_caller_identity" {
    defaults = {
      account_id = "111122223333"
    }
  }
}

mock_provider "random" {}

# A plan leaves every computed attribute unknown, so each value an assertion reads is fixed here for the plan phase
override_resource {
  target          = random_id.s3_suffix
  values          = { hex = "0a1b2c3d" }
  override_during = plan
}

override_resource {
  target = aws_rds_cluster.this
  values = {
    cluster_resource_id = "cluster-ABCDEFGHIJKLMNOPQRSTUVW"
    endpoint            = "acme-litellm-test.cluster-cdefghijklmn.us-east-1.rds.amazonaws.com"
    reader_endpoint     = "acme-litellm-test.cluster-ro-cdefghijklmn.us-east-1.rds.amazonaws.com"
    port                = 5432
  }
  override_during = plan
}

override_resource {
  target = aws_elasticache_replication_group.this
  values = {
    primary_endpoint_address = "acme-litellm-test-redis.abc123.0001.use1.cache.amazonaws.com"
    port                     = 6379
  }
  override_during = plan
}

override_resource {
  target          = aws_secretsmanager_secret.master_key
  values          = { arn = "arn:aws:secretsmanager:us-east-1:111122223333:secret:acme-litellm-test-master-key-AAAAAA" }
  override_during = plan
}

override_resource {
  target          = aws_secretsmanager_secret.database_url
  values          = { arn = "arn:aws:secretsmanager:us-east-1:111122223333:secret:acme-litellm-test-database-url-DDDDDD" }
  override_during = plan
}

variables {
  region              = "us-east-1"
  tenant              = "acme"
  env                 = "test"
  azs                 = ["us-east-1a", "us-east-1b"]
  image               = "ghcr.io/example/betterlite:v1"
  allow_plaintext_alb = true
}

run "pool_is_off_by_default" {
  command = plan

  assert {
    condition = alltrue([
      for absent in ["LITELLM_PGBOUNCER_ENABLED", "LITELLM_PGBOUNCER_MAX_DB_CONNECTIONS", "LITELLM_PGBOUNCER_MAX_CLIENT_CONN"] :
      !contains([for entry in jsondecode(aws_ecs_task_definition.betterlite.container_definitions)[0].environment : entry.name], absent)
    ])
    error_message = "connection_pool_enabled defaults to false, so no pool variable may reach the container."
  }
}

run "pool_limits_reach_the_container" {
  command = plan

  variables {
    connection_pool_enabled = true
    pool_max_db_connections = 25
    pool_max_client_conn    = 750
  }

  assert {
    condition = alltrue([
      for expected in [
        "LITELLM_PGBOUNCER_ENABLED=true",
        "LITELLM_PGBOUNCER_MAX_DB_CONNECTIONS=25",
        "LITELLM_PGBOUNCER_MAX_CLIENT_CONN=750",
        ] : contains([
          for entry in jsondecode(aws_ecs_task_definition.betterlite.container_definitions)[0].environment :
          "${entry.name}=${entry.value}"
      ], expected)
    ])
    error_message = "Turning the pool on must set the three PgBouncer variables with the configured limits."
  }
}

run "pool_needs_a_database" {
  command = plan

  variables {
    connection_pool_enabled = true
    create_database         = false
  }

  expect_failures = [
    aws_ecs_task_definition.betterlite,
  ]
}

run "pool_with_a_supplied_database_is_allowed" {
  command = plan

  variables {
    connection_pool_enabled = true
    create_database         = false
    database_url            = "postgresql://litellm:pw@db.internal:5432/litellm"
  }

  assert {
    condition = alltrue([
      for expected in [
        "LITELLM_PGBOUNCER_ENABLED=true",
        "LITELLM_PGBOUNCER_MAX_DB_CONNECTIONS=20",
        "LITELLM_PGBOUNCER_MAX_CLIENT_CONN=1000",
        ] : contains([
          for entry in jsondecode(aws_ecs_task_definition.betterlite.container_definitions)[0].environment :
          "${entry.name}=${entry.value}"
      ], expected)
    ])
    error_message = "A supplied database satisfies the pool precondition, and the default limits must land in the environment."
  }
}

run "pool_db_connections_below_one_fails_its_validation" {
  command = plan

  variables {
    pool_max_db_connections = 0
  }

  expect_failures = [
    var.pool_max_db_connections,
  ]
}

run "pool_client_connections_below_one_fails_its_validation" {
  command = plan

  variables {
    pool_max_client_conn = 0
  }

  expect_failures = [
    var.pool_max_client_conn,
  ]
}
