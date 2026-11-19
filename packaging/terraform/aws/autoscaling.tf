resource "aws_appautoscaling_target" "betterlite" {
  count              = var.autoscaling_enabled ? 1 : 0
  service_namespace  = "ecs"
  resource_id        = "service/${aws_ecs_cluster.this.name}/${aws_ecs_service.betterlite.name}"
  scalable_dimension = "ecs:service:DesiredCount"
  min_capacity       = var.min_capacity
  max_capacity       = var.max_capacity
}

resource "aws_appautoscaling_policy" "betterlite_cpu" {
  count              = var.autoscaling_enabled ? 1 : 0
  name               = "${local.name}-cpu"
  policy_type        = "TargetTrackingScaling"
  service_namespace  = aws_appautoscaling_target.betterlite[0].service_namespace
  resource_id        = aws_appautoscaling_target.betterlite[0].resource_id
  scalable_dimension = aws_appautoscaling_target.betterlite[0].scalable_dimension

  target_tracking_scaling_policy_configuration {
    predefined_metric_specification {
      predefined_metric_type = "ECSServiceAverageCPUUtilization"
    }
    target_value = var.cpu_target
  }
}

resource "aws_appautoscaling_policy" "betterlite_memory" {
  count              = var.autoscaling_enabled && var.memory_target > 0 ? 1 : 0
  name               = "${local.name}-memory"
  policy_type        = "TargetTrackingScaling"
  service_namespace  = aws_appautoscaling_target.betterlite[0].service_namespace
  resource_id        = aws_appautoscaling_target.betterlite[0].resource_id
  scalable_dimension = aws_appautoscaling_target.betterlite[0].scalable_dimension

  target_tracking_scaling_policy_configuration {
    predefined_metric_specification {
      predefined_metric_type = "ECSServiceAverageMemoryUtilization"
    }
    target_value = var.memory_target
  }
}

resource "aws_appautoscaling_policy" "betterlite_requests" {
  count              = var.autoscaling_enabled && var.target_requests_per_second > 0 ? 1 : 0
  name               = "${local.name}-requests"
  policy_type        = "TargetTrackingScaling"
  service_namespace  = aws_appautoscaling_target.betterlite[0].service_namespace
  resource_id        = aws_appautoscaling_target.betterlite[0].resource_id
  scalable_dimension = aws_appautoscaling_target.betterlite[0].scalable_dimension

  target_tracking_scaling_policy_configuration {
    predefined_metric_specification {
      predefined_metric_type = "ALBRequestCountPerTarget"
      resource_label         = "${aws_lb.this.arn_suffix}/${aws_lb_target_group.betterlite.arn_suffix}"
    }
    # ALBRequestCountPerTarget is a per-minute count
    target_value = var.target_requests_per_second * 60
  }
}
