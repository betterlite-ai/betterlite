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
  target          = aws_lb_target_group.betterlite
  values          = { arn = "arn:aws:elasticloadbalancing:us-east-1:111122223333:targetgroup/acme-litellm-test/0123456789abcdef" }
  override_during = plan
}

override_resource {
  target          = aws_security_group.alb
  values          = { id = "sg-0fedcba9876543210" }
  override_during = plan
}

variables {
  region = "us-east-1"
  tenant = "acme"
  env    = "test"
  azs    = ["us-east-1a", "us-east-1b"]
  image  = "ghcr.io/example/betterlite:v1"
}

run "plaintext_refused_without_a_certificate" {
  command = plan

  expect_failures = [
    aws_lb_listener.http,
  ]
}

run "plaintext_serves_the_single_target_group" {
  command = plan

  variables {
    allow_plaintext_alb = true
  }

  assert {
    condition = alltrue([
      aws_lb.this.name == local.name,
      aws_lb.this.load_balancer_type == "application",
      aws_lb.this.internal == false,
      aws_lb.this.idle_timeout == 120,
      toset(aws_lb.this.security_groups) == toset([aws_security_group.alb.id]),
      length(aws_lb_listener.https) == 0,
    ])
    error_message = "Without a certificate and with plaintext allowed, the module runs one internet-facing ALB with a 120 second idle timeout and no HTTPS listener."
  }

  assert {
    condition = alltrue([
      aws_lb_listener.http.port == 80,
      aws_lb_listener.http.protocol == "HTTP",
      aws_lb_listener.http.default_action[0].type == "forward",
      aws_lb_listener.http.default_action[0].target_group_arn == aws_lb_target_group.betterlite.arn,
    ])
    error_message = "The HTTP listener must forward every request to the single betterlite target group."
  }

  assert {
    condition = alltrue([
      aws_lb_target_group.betterlite.port == 4000,
      aws_lb_target_group.betterlite.protocol == "HTTP",
      aws_lb_target_group.betterlite.target_type == "ip",
      aws_lb_target_group.betterlite.health_check[0].path == "/health/readiness",
      aws_lb_target_group.betterlite.health_check[0].matcher == "200-299",
      aws_lb_target_group.betterlite.health_check[0].interval == 30,
      aws_lb_target_group.betterlite.health_check[0].timeout == 10,
      aws_lb_target_group.betterlite.health_check[0].healthy_threshold == 2,
      aws_lb_target_group.betterlite.health_check[0].unhealthy_threshold == 3,
      aws_lb_target_group.betterlite.deregistration_delay == 30,
    ])
    error_message = "The target group must be IP-based on port 4000 and poll /health/readiness on the module's schedule."
  }
}

run "certificate_adds_https_and_redirects_http" {
  command = plan

  variables {
    acm_certificate_arn = "arn:aws:acm:us-east-1:111122223333:certificate/abcdef01-2345-6789-abcd-ef0123456789"
  }

  assert {
    condition = alltrue([
      aws_lb_listener.http.default_action[0].type == "redirect",
      aws_lb_listener.http.default_action[0].redirect[0].port == "443",
      aws_lb_listener.http.default_action[0].redirect[0].protocol == "HTTPS",
      aws_lb_listener.http.default_action[0].redirect[0].status_code == "HTTP_301",
      aws_lb_listener.http.default_action[0].target_group_arn == null,
    ])
    error_message = "With TLS on, port 80 must answer with a 301 to https on 443 and forward no traffic itself."
  }

  assert {
    condition = alltrue([
      length(aws_lb_listener.https) == 1,
      aws_lb_listener.https[0].port == 443,
      aws_lb_listener.https[0].protocol == "HTTPS",
      aws_lb_listener.https[0].ssl_policy == "ELBSecurityPolicy-TLS13-1-2-2021-06",
      aws_lb_listener.https[0].certificate_arn == var.acm_certificate_arn,
      aws_lb_listener.https[0].default_action[0].type == "forward",
      aws_lb_listener.https[0].default_action[0].target_group_arn == aws_lb_target_group.betterlite.arn,
    ])
    error_message = "The 443 listener must terminate TLS with the supplied certificate and TLS 1.2/1.3 policy and forward to the target group."
  }
}

run "supplied_subnets_hold_the_alb" {
  command = plan

  variables {
    vpc_id              = "vpc-00000000000000001"
    public_subnet_ids   = ["subnet-pub-a", "subnet-pub-b"]
    private_subnet_ids  = ["subnet-priv-a", "subnet-priv-b"]
    allow_plaintext_alb = true
  }

  assert {
    condition = alltrue([
      toset(aws_lb.this.subnets) == toset(var.public_subnet_ids),
      aws_lb_target_group.betterlite.vpc_id == var.vpc_id,
      aws_lb.this.internal == false,
      aws_lb.this.idle_timeout == 120,
    ])
    error_message = "With vpc_id set the ALB must land in the supplied public subnets and the target group in the supplied VPC."
  }
}

run "one_public_subnet_fails_at_plan" {
  command = plan

  variables {
    vpc_id              = "vpc-00000000000000001"
    public_subnet_ids   = ["subnet-pub-a"]
    private_subnet_ids  = ["subnet-priv-a", "subnet-priv-b"]
    allow_plaintext_alb = true
  }

  expect_failures = [
    aws_lb.this,
  ]
}
