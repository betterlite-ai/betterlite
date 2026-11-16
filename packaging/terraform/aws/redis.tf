resource "aws_elasticache_subnet_group" "this" {
  count      = var.create_redis ? 1 : 0
  name       = "${local.name}-redis"
  subnet_ids = local.private_subnet_ids

  tags = local.tags
}

resource "aws_elasticache_replication_group" "this" {
  count                = var.create_redis ? 1 : 0
  replication_group_id = "${local.name}-redis"
  description          = "betterlite ElastiCache Redis"

  engine               = "redis"
  engine_version       = "7.1"
  node_type            = var.redis_node_type
  num_cache_clusters   = 1 + var.redis_num_replicas
  parameter_group_name = "default.redis7"
  port                 = 6379

  subnet_group_name  = aws_elasticache_subnet_group.this[0].name
  security_group_ids = [aws_security_group.redis[0].id]

  automatic_failover_enabled = var.redis_num_replicas >= 1
  multi_az_enabled           = var.redis_num_replicas >= 1
  at_rest_encryption_enabled = true
  transit_encryption_enabled = true

  apply_immediately = true

  tags = local.tags
}
