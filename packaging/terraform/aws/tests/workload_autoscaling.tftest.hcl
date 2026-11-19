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
  target          = aws_lb.this
  values          = { arn_suffix = "app/acme-litellm-test/1234567890abcdef" }
  override_during = plan
}

override_resource {
  target          = aws_lb_target_group.betterlite
  values          = { arn_suffix = "targetgroup/acme-litellm-test/0123456789abcdef" }
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

run "defaults_track_cpu_and_memory" {
  command = plan

  assert {
    condition = alltrue([
      length(aws_appautoscaling_target.betterlite) == 1,
      aws_appautoscaling_target.betterlite[0].service_namespace == "ecs",
      aws_appautoscaling_target.betterlite[0].scalable_dimension == "ecs:service:DesiredCount",
      aws_appautoscaling_target.betterlite[0].resource_id == "service/${aws_ecs_cluster.this.name}/${aws_ecs_service.betterlite.name}",
      aws_appautoscaling_target.betterlite[0].min_capacity == 1,
      aws_appautoscaling_target.betterlite[0].max_capacity == 10,
    ])
    error_message = "Autoscaling is on by default, so one scalable target on the service must allow 1 to 10 tasks."
  }

  assert {
    condition = alltrue([
      length(aws_appautoscaling_policy.betterlite_cpu) == 1,
      length(aws_appautoscaling_policy.betterlite_memory) == 1,
      length(aws_appautoscaling_policy.betterlite_requests) == 0,
      aws_appautoscaling_policy.betterlite_cpu[0].policy_type == "TargetTrackingScaling",
      aws_appautoscaling_policy.betterlite_cpu[0].target_tracking_scaling_policy_configuration[0].predefined_metric_specification[0].predefined_metric_type == "ECSServiceAverageCPUUtilization",
      tostring(aws_appautoscaling_policy.betterlite_cpu[0].target_tracking_scaling_policy_configuration[0].target_value) == "70",
      aws_appautoscaling_policy.betterlite_memory[0].target_tracking_scaling_policy_configuration[0].predefined_metric_specification[0].predefined_metric_type == "ECSServiceAverageMemoryUtilization",
      tostring(aws_appautoscaling_policy.betterlite_memory[0].target_tracking_scaling_policy_configuration[0].target_value) == "80",
    ])
    error_message = "The default policies are CPU at 70 and memory at 80, with no request policy while target_requests_per_second is 0."
  }
}

run "memory_policy_off_at_zero" {
  command = plan

  variables {
    memory_target = 0
  }

  assert {
    condition = alltrue([
      length(aws_appautoscaling_policy.betterlite_cpu) == 1,
      length(aws_appautoscaling_policy.betterlite_memory) == 0,
    ])
    error_message = "memory_target = 0 must drop the memory policy and keep the CPU one."
  }
}

run "requests_policy_scales_on_the_target_group" {
  command = plan

  variables {
    target_requests_per_second = 5
  }

  assert {
    condition = alltrue([
      length(aws_appautoscaling_policy.betterlite_requests) == 1,
      length(aws_appautoscaling_policy.betterlite_cpu) == 1,
      length(aws_appautoscaling_policy.betterlite_memory) == 1,
      aws_appautoscaling_policy.betterlite_requests[0].target_tracking_scaling_policy_configuration[0].predefined_metric_specification[0].predefined_metric_type == "ALBRequestCountPerTarget",
      aws_appautoscaling_policy.betterlite_requests[0].target_tracking_scaling_policy_configuration[0].predefined_metric_specification[0].resource_label == "${aws_lb.this.arn_suffix}/${aws_lb_target_group.betterlite.arn_suffix}",
      tostring(aws_appautoscaling_policy.betterlite_requests[0].target_tracking_scaling_policy_configuration[0].target_value) == "300",
    ])
    error_message = "A request policy counts per minute against the load balancer and target group, so 5 requests per second targets 300."
  }
}

run "requests_policy_without_the_memory_policy" {
  command = plan

  variables {
    memory_target              = 0
    target_requests_per_second = 2
  }

  assert {
    condition = alltrue([
      length(aws_appautoscaling_policy.betterlite_cpu) == 1,
      length(aws_appautoscaling_policy.betterlite_memory) == 0,
      length(aws_appautoscaling_policy.betterlite_requests) == 1,
      tostring(aws_appautoscaling_policy.betterlite_requests[0].target_tracking_scaling_policy_configuration[0].target_value) == "120",
    ])
    error_message = "The CPU and request policies must combine while the memory policy stays off."
  }
}

run "autoscaling_off_creates_no_target_or_policy" {
  command = plan

  variables {
    autoscaling_enabled = false
    desired_count       = 3
  }

  assert {
    condition = alltrue([
      length(aws_appautoscaling_target.betterlite) == 0,
      length(aws_appautoscaling_policy.betterlite_cpu) == 0,
      length(aws_appautoscaling_policy.betterlite_memory) == 0,
      length(aws_appautoscaling_policy.betterlite_requests) == 0,
      aws_ecs_service.betterlite.desired_count == 3,
    ])
    error_message = "With autoscaling off nothing is created and the service keeps desired_count."
  }
}

run "capacity_bounds_reach_the_target" {
  command = plan

  variables {
    min_capacity = 2
    max_capacity = 7
  }

  assert {
    condition = alltrue([
      aws_appautoscaling_target.betterlite[0].min_capacity == 2,
      aws_appautoscaling_target.betterlite[0].max_capacity == 7,
    ])
    error_message = "min_capacity and max_capacity must set the scalable target's bounds."
  }
}
