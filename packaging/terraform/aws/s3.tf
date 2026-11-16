resource "random_id" "s3_suffix" {
  byte_length = 4
}

resource "aws_s3_bucket" "this" {
  bucket = "${local.name}-${random_id.s3_suffix.hex}"

  # Default false makes a destroy over a non-empty bucket fail, so cached responses, archived request logs and uploaded files stay until an operator says otherwise
  force_destroy = var.s3_force_destroy

  tags = local.tags
}

resource "aws_s3_bucket_versioning" "this" {
  bucket = aws_s3_bucket.this.id

  versioning_configuration {
    status = "Enabled"
  }
}

resource "aws_s3_bucket_server_side_encryption_configuration" "this" {
  bucket = aws_s3_bucket.this.id

  rule {
    apply_server_side_encryption_by_default {
      sse_algorithm = "AES256"
    }
  }
}

resource "aws_s3_bucket_public_access_block" "this" {
  bucket = aws_s3_bucket.this.id

  block_public_acls       = true
  block_public_policy     = true
  ignore_public_acls      = true
  restrict_public_buckets = true
}

data "aws_iam_policy_document" "s3_access" {
  statement {
    actions = [
      "s3:ListBucket",
      "s3:GetBucketLocation",
    ]
    resources = [aws_s3_bucket.this.arn]
  }

  statement {
    actions = [
      "s3:GetObject",
      "s3:PutObject",
      "s3:DeleteObject",
      "s3:AbortMultipartUpload",
      "s3:ListMultipartUploadParts",
    ]
    resources = ["${aws_s3_bucket.this.arn}/*"]
  }
}

resource "aws_iam_policy" "s3_access" {
  name   = "${local.name}-s3-access"
  policy = data.aws_iam_policy_document.s3_access.json

  tags = local.tags
}

resource "aws_iam_role_policy_attachment" "task_s3_access" {
  role       = aws_iam_role.task.name
  policy_arn = aws_iam_policy.s3_access.arn
}

# The object only exists once there is a config to upload, so a stack with no proxy_config starts on the image's defaults
resource "aws_s3_object" "proxy_config" {
  count = length(keys(var.proxy_config)) > 0 ? 1 : 0

  bucket       = aws_s3_bucket.this.id
  key          = "config/config.yaml"
  content      = yamlencode(var.proxy_config)
  content_type = "application/yaml"
}
