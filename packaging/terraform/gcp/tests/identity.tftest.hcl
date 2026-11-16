mock_provider "google" {
  override_during = plan

  # The SQL instance checks that private_network is a VPC resource id or self link
  mock_resource "google_compute_network" {
    defaults = {
      id        = "projects/test-project/global/networks/tenant-test-betterlite"
      self_link = "https://www.googleapis.com/compute/v1/projects/test-project/global/networks/tenant-test-betterlite"
    }
  }
}

mock_provider "random" {
  override_during = plan

  mock_resource "random_password" {}
  mock_resource "random_id" {
    defaults = {
      hex = "deadbeef"
    }
  }
}

variables {
  project_id         = "test-project"
  tenant             = "tenant"
  env                = "test"
  litellm_master_key = "sk-test-master-key"
}

run "master_key_secret_always_exists" {
  command = plan

  assert {
    condition = alltrue([
      google_secret_manager_secret.master_key.secret_id == "tenant-test-betterlite-master-key",
      google_secret_manager_secret.master_key.labels["managed_by"] == "terraform",
      length(google_secret_manager_secret.master_key.replication) == 1,
      google_secret_manager_secret_version.master_key.secret_data == var.litellm_master_key,
      local.license_set == false,
      length(google_secret_manager_secret.license) == 0,
      length(google_secret_manager_secret_version.license) == 0,
      length(google_secret_manager_secret_iam_member.license) == 0,
      local.ui_password_set == false,
      length(google_secret_manager_secret.ui_password) == 0,
      length(google_secret_manager_secret_version.ui_password) == 0,
      length(google_secret_manager_secret_iam_member.ui_password) == 0,
    ])
    error_message = "The master key secret and version must always exist and the license and ui_password secrets must stay absent while their inputs are empty."
  }
}

run "license_and_ui_password_secrets_exist_when_set" {
  command = plan

  variables {
    litellm_license = "license-abc"
    ui_password     = "ui-secret"
  }

  assert {
    condition = alltrue([
      local.license_set == true,
      local.ui_password_set == true,
      length(google_secret_manager_secret.license) == 1,
      google_secret_manager_secret.license[0].secret_id == "tenant-test-betterlite-license",
      length(google_secret_manager_secret_version.license) == 1,
      google_secret_manager_secret_version.license[0].secret_data == var.litellm_license,
      length(google_secret_manager_secret.ui_password) == 1,
      google_secret_manager_secret.ui_password[0].secret_id == "tenant-test-betterlite-ui-password",
      length(google_secret_manager_secret_version.ui_password) == 1,
      google_secret_manager_secret_version.ui_password[0].secret_data == var.ui_password,
    ])
    error_message = "A non-empty litellm_license and ui_password must each create a secret and its version holding the caller's value."
  }
}

run "runtime_account_gets_accessor_on_every_secret" {
  command = plan

  variables {
    litellm_license = "license-abc"
    ui_password     = "ui-secret"
    extra_secrets = {
      OPENAI_API_KEY = "projects/test-project/secrets/openai-api-key"
    }
  }

  assert {
    condition = alltrue([
      google_secret_manager_secret_iam_member.master_key.secret_id == google_secret_manager_secret.master_key.id,
      google_secret_manager_secret_iam_member.master_key.role == "roles/secretmanager.secretAccessor",
      google_secret_manager_secret_iam_member.master_key.member == "serviceAccount:${google_service_account.runtime.email}",
      google_secret_manager_secret_iam_member.db_password.secret_id == google_secret_manager_secret.db_password.id,
      google_secret_manager_secret_iam_member.db_password.role == "roles/secretmanager.secretAccessor",
      google_secret_manager_secret_iam_member.db_password.member == "serviceAccount:${google_service_account.runtime.email}",
      length(google_secret_manager_secret_iam_member.license) == 1,
      google_secret_manager_secret_iam_member.license[0].secret_id == google_secret_manager_secret.license[0].id,
      length(google_secret_manager_secret_iam_member.ui_password) == 1,
      google_secret_manager_secret_iam_member.ui_password[0].secret_id == google_secret_manager_secret.ui_password[0].id,
      length(google_secret_manager_secret_iam_member.extra_secrets) == 1,
      alltrue([
        for id, binding in google_secret_manager_secret_iam_member.extra_secrets :
        binding.secret_id == id &&
        binding.role == "roles/secretmanager.secretAccessor" &&
        binding.member == "serviceAccount:${google_service_account.runtime.email}"
      ]),
    ])
    error_message = "The runtime account must hold roles/secretmanager.secretAccessor on the master key, the database password, the optional secrets and every extra secret."
  }
}

run "extra_secrets_bind_once_per_distinct_secret" {
  command = plan

  variables {
    extra_secrets = {
      OPENAI_API_KEY       = "projects/test-project/secrets/openai-api-key"
      OPENAI_API_KEY_ALIAS = "projects/test-project/secrets/openai-api-key"
      ANTHROPIC_API_KEY    = "projects/test-project/secrets/anthropic-api-key"
    }
  }

  assert {
    condition = alltrue([
      length(google_secret_manager_secret_iam_member.extra_secrets) == 2,
      contains(keys(google_secret_manager_secret_iam_member.extra_secrets), "projects/test-project/secrets/openai-api-key"),
      contains(keys(google_secret_manager_secret_iam_member.extra_secrets), "projects/test-project/secrets/anthropic-api-key"),
    ])
    error_message = "Two environment names that point at the same secret must produce one binding, keyed by the secret id."
  }
}

run "runtime_account_gets_the_project_roles" {
  command = plan

  assert {
    condition = alltrue([
      google_project_iam_member.runtime_cloudsql.project == "test-project",
      google_project_iam_member.runtime_cloudsql.role == "roles/cloudsql.client",
      google_project_iam_member.runtime_cloudsql.member == "serviceAccount:${google_service_account.runtime.email}",
      google_project_iam_member.runtime_log_writer.project == "test-project",
      google_project_iam_member.runtime_log_writer.role == "roles/logging.logWriter",
      google_project_iam_member.runtime_log_writer.member == "serviceAccount:${google_service_account.runtime.email}",
    ])
    error_message = "The runtime account must get roles/cloudsql.client and roles/logging.logWriter on the project."
  }
}

run "runtime_account_id_fits_the_thirty_character_limit" {
  command = plan

  assert {
    condition     = google_service_account.runtime.account_id == "tenant-test-betterlite-runtime" && length(google_service_account.runtime.account_id) <= 30
    error_message = "The runtime account id must be the module name plus -runtime and must stay within 30 characters."
  }
}

run "runtime_account_id_is_cut_for_long_names" {
  command = plan

  variables {
    tenant = "abcdefghijklmnopqrstu"
    env    = "b"
  }

  assert {
    condition = alltrue([
      length(google_service_account.runtime.account_id) <= 30,
      google_service_account.runtime.account_id == "abcdefghijklmnopqrstu-runtime",
    ])
    error_message = "A long shared name must be cut before the -runtime suffix, without keeping the hyphen the cut lands on."
  }
}

run "data_bucket_is_locked_down" {
  command = plan

  assert {
    condition = alltrue([
      google_storage_bucket.data.name == "test-project-tenant-test-betterlite-deadbeef",
      google_storage_bucket.data.location == "us-central1",
      google_storage_bucket.data.uniform_bucket_level_access == true,
      google_storage_bucket.data.public_access_prevention == "enforced",
      google_storage_bucket.data.force_destroy == false,
      google_storage_bucket.data.versioning[0].enabled == true,
      try(length(google_storage_bucket.data.lifecycle_rule), 0) == 0,
      google_storage_bucket_iam_member.data_runtime.bucket == google_storage_bucket.data.name,
      google_storage_bucket_iam_member.data_runtime.role == "roles/storage.objectAdmin",
      google_storage_bucket_iam_member.data_runtime.member == "serviceAccount:${google_service_account.runtime.email}",
      local.proxy_config_enabled == false,
      local.bucket_location == "us-central1",
      length(google_storage_bucket.config) == 0,
      length(google_storage_bucket_object.config) == 0,
      length(google_storage_bucket_iam_member.config_runtime) == 0,
    ])
    error_message = "The data bucket must be uniform, versioned and public-access-prevented with no lifecycle rule by default, and no config bucket may exist without a proxy_config."
  }
}

run "config_bucket_and_object_exist_with_a_proxy_config" {
  command = plan

  variables {
    proxy_config = {
      model_list = [{
        model_name     = "gpt-4o"
        litellm_params = { model = "openai/gpt-4o" }
      }]
    }
  }

  assert {
    condition = alltrue([
      local.proxy_config_enabled == true,
      local.proxy_config_hash == md5(yamlencode(var.proxy_config)),
      length(google_storage_bucket.config) == 1,
      google_storage_bucket.config[0].name == "test-project-tenant-test-betterlite-config-deadbeef",
      google_storage_bucket.config[0].location == "us-central1",
      google_storage_bucket.config[0].uniform_bucket_level_access == true,
      google_storage_bucket.config[0].public_access_prevention == "enforced",
      google_storage_bucket.config[0].force_destroy == false,
      google_storage_bucket.config[0].versioning[0].enabled == true,
      length(google_storage_bucket_object.config) == 1,
      google_storage_bucket_object.config[0].name == "config.yaml",
      google_storage_bucket_object.config[0].bucket == google_storage_bucket.config[0].name,
      google_storage_bucket_object.config[0].content == yamlencode(var.proxy_config),
      google_storage_bucket_object.config[0].content_type == "application/yaml",
      length(google_storage_bucket_iam_member.config_runtime) == 1,
      google_storage_bucket_iam_member.config_runtime[0].bucket == google_storage_bucket.config[0].name,
      google_storage_bucket_iam_member.config_runtime[0].role == "roles/storage.objectViewer",
      google_storage_bucket_iam_member.config_runtime[0].member == "serviceAccount:${google_service_account.runtime.email}",
    ])
    error_message = "A proxy_config must create the hardened config bucket, the config.yaml object holding the encoded map, and the objectViewer binding for the runtime account."
  }
}

run "data_retention_days_adds_a_delete_rule" {
  command = plan

  variables {
    data_retention_days = 30
  }

  assert {
    condition = alltrue([
      length(google_storage_bucket.data.lifecycle_rule) == 1,
      alltrue([
        for rule in google_storage_bucket.data.lifecycle_rule :
        rule.condition[0].age == 30 && rule.action[0].type == "Delete"
      ]),
    ])
    error_message = "A positive data_retention_days must add one delete lifecycle rule with that age to the data bucket."
  }
}

run "bucket_force_destroy_and_location_reach_both_buckets" {
  command = plan

  variables {
    bucket_force_destroy = true
    bucket_location      = "EU"
    proxy_config         = { model_list = [] }
  }

  assert {
    condition = alltrue([
      google_storage_bucket.data.force_destroy == true,
      google_storage_bucket.data.location == "EU",
      google_storage_bucket.config[0].force_destroy == true,
      google_storage_bucket.config[0].location == "EU",
      local.bucket_location == "EU",
    ])
    error_message = "bucket_force_destroy and bucket_location must reach both buckets."
  }
}

run "no_runtime_keeps_the_stores_but_leaves_out_the_config_bucket" {
  command = plan

  variables {
    create_runtime = false
    proxy_config   = { model_list = [] }
  }

  assert {
    condition = alltrue([
      local.proxy_config_enabled == false,
      length(google_storage_bucket.config) == 0,
      length(google_storage_bucket_object.config) == 0,
      length(google_storage_bucket_iam_member.config_runtime) == 0,
      google_storage_bucket.data.name == "test-project-tenant-test-betterlite-deadbeef",
      google_storage_bucket_iam_member.data_runtime.role == "roles/storage.objectAdmin",
      google_service_account.runtime.account_id == "tenant-test-betterlite-runtime",
      google_secret_manager_secret.master_key.secret_id == "tenant-test-betterlite-master-key",
    ])
    error_message = "create_runtime = false must keep the data bucket, the runtime account and the secrets but build no config bucket."
  }
}

run "negative_data_retention_days_fails" {
  command = plan

  variables {
    data_retention_days = -1
  }

  expect_failures = [var.data_retention_days]
}
