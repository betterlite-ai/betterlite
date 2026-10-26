use super::descriptors::FieldDescriptor;

pub const EMAIL_DESCRIPTORS: [FieldDescriptor; 9] = [
        FieldDescriptor::new("SMTP_HOST", "SMTP_HOST", "SMTP_HOST", false, None),
        FieldDescriptor::new("SMTP_PORT", "SMTP_PORT", "SMTP_PORT", false, Some("587")),
        FieldDescriptor::new("SMTP_TLS", "SMTP_TLS", "SMTP_TLS", false, Some("True")),
        FieldDescriptor::new("SMTP_USERNAME", "SMTP_USERNAME", "SMTP_USERNAME", true, None),
        FieldDescriptor::new("SMTP_PASSWORD", "SMTP_PASSWORD", "SMTP_PASSWORD", true, None),
        FieldDescriptor::new("SMTP_SENDER_EMAIL", "SMTP_SENDER_EMAIL", "SMTP_SENDER_EMAIL", false, None),
        FieldDescriptor::new("TEST_EMAIL_ADDRESS", "TEST_EMAIL_ADDRESS", "TEST_EMAIL_ADDRESS", false, None),
        FieldDescriptor::new("EMAIL_LOGO_URL", "EMAIL_LOGO_URL", "EMAIL_LOGO_URL", false, None),
        FieldDescriptor::new("EMAIL_SUPPORT_CONTACT", "EMAIL_SUPPORT_CONTACT", "EMAIL_SUPPORT_CONTACT", false, None),
    ];

pub const SLACK_DESCRIPTORS: [FieldDescriptor; 1] = [
    FieldDescriptor::new("SLACK_WEBHOOK_URL", "SLACK_WEBHOOK_URL", "SLACK_WEBHOOK_URL", true, None),
];

pub const MS_TEAMS_DESCRIPTORS: [FieldDescriptor; 1] = [
    FieldDescriptor::new("MS_TEAMS_WEBHOOK_URL", "MS_TEAMS_WEBHOOK_URL", "MS_TEAMS_WEBHOOK_URL", true, None),
];

#[must_use]
pub const fn email_descriptors() -> &'static [FieldDescriptor; 9] {
    &EMAIL_DESCRIPTORS
}

#[must_use]
pub const fn slack_descriptors() -> &'static [FieldDescriptor; 1] {
    &SLACK_DESCRIPTORS
}

#[must_use]
pub const fn ms_teams_descriptors() -> &'static [FieldDescriptor; 1] {
    &MS_TEAMS_DESCRIPTORS
}
