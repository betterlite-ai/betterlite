locals {
  name = "${var.tenant}-litellm-${var.env}"

  tags = merge(
    {
      "litellm:stack" = local.name
      "managed-by"    = "terraform"
    },
    var.tags,
  )

  create_vpc         = var.vpc_id == ""
  vpc_id             = local.create_vpc ? aws_vpc.this[0].id : var.vpc_id
  public_subnet_ids  = local.create_vpc ? aws_subnet.public[*].id : var.public_subnet_ids
  private_subnet_ids = local.create_vpc ? aws_subnet.private[*].id : var.private_subnet_ids

  task_security_group_ids = concat([aws_security_group.tasks.id], var.additional_task_security_group_ids)

  # nonsensitive() wraps only the emptiness test, so a supplied URL stays redacted in plans
  byo_database     = !var.create_database && nonsensitive(var.database_url != "")
  byo_redis        = !var.create_redis && nonsensitive(var.redis_url != "")
  database_enabled = var.create_database || local.byo_database
  redis_enabled    = var.create_redis || local.byo_redis

  # Aurora and ElastiCache subnet groups both need subnets in two AZs
  managed_stores_need_two_azs = var.create_database || var.create_redis
}
