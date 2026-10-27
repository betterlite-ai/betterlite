use super::FieldDescriptor;

// SMTP_PORT and SMTP_TLS carry the defaults the mail sender applies, so the settings page shows what mail uses
pub const EMAIL_DESCRIPTORS: [FieldDescriptor; 9] = [
    FieldDescriptor {
        field_name: "SMTP_HOST",
        db_key: "SMTP_HOST",
        env_var: "SMTP_HOST",
        is_secret: false,
        default: None,
    },
    FieldDescriptor {
        field_name: "SMTP_PORT",
        db_key: "SMTP_PORT",
        env_var: "SMTP_PORT",
        is_secret: false,
        default: Some("587"),
    },
    FieldDescriptor {
        field_name: "SMTP_TLS",
        db_key: "SMTP_TLS",
        env_var: "SMTP_TLS",
        is_secret: false,
        default: Some("True"),
    },
    FieldDescriptor {
        field_name: "SMTP_USERNAME",
        db_key: "SMTP_USERNAME",
        env_var: "SMTP_USERNAME",
        is_secret: true,
        default: None,
    },
    FieldDescriptor {
        field_name: "SMTP_PASSWORD",
        db_key: "SMTP_PASSWORD",
        env_var: "SMTP_PASSWORD",
        is_secret: true,
        default: None,
    },
    FieldDescriptor {
        field_name: "SMTP_SENDER_EMAIL",
        db_key: "SMTP_SENDER_EMAIL",
        env_var: "SMTP_SENDER_EMAIL",
        is_secret: false,
        default: None,
    },
    FieldDescriptor {
        field_name: "TEST_EMAIL_ADDRESS",
        db_key: "TEST_EMAIL_ADDRESS",
        env_var: "TEST_EMAIL_ADDRESS",
        is_secret: false,
        default: None,
    },
    FieldDescriptor {
        field_name: "EMAIL_LOGO_URL",
        db_key: "EMAIL_LOGO_URL",
        env_var: "EMAIL_LOGO_URL",
        is_secret: false,
        default: None,
    },
    FieldDescriptor {
        field_name: "EMAIL_SUPPORT_CONTACT",
        db_key: "EMAIL_SUPPORT_CONTACT",
        env_var: "EMAIL_SUPPORT_CONTACT",
        is_secret: false,
        default: None,
    },
];

pub const SLACK_DESCRIPTORS: [FieldDescriptor; 1] = [FieldDescriptor {
    field_name: "SLACK_WEBHOOK_URL",
    db_key: "SLACK_WEBHOOK_URL",
    env_var: "SLACK_WEBHOOK_URL",
    is_secret: true,
    default: None,
}];

pub const MS_TEAMS_DESCRIPTORS: [FieldDescriptor; 1] = [FieldDescriptor {
    field_name: "MS_TEAMS_WEBHOOK_URL",
    db_key: "MS_TEAMS_WEBHOOK_URL",
    env_var: "MS_TEAMS_WEBHOOK_URL",
    is_secret: true,
    default: None,
}];
