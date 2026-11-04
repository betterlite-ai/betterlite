use crate::api::responses_shapes::FunctionCallStatus;

record! { Computer: ComputerTag = "computer_call" {
    id: String,
    call_id: String,
    pending_safety_checks: Vec<PendingSafetyCheck>,
    status: FunctionCallStatus,
    action: Option<super::unions::ComputerActionValue>,
    actions: Option<Vec<super::unions::ComputerActionValue>>,
} }
record! { PendingSafetyCheck {
    id: String,
    code: Option<String>,
    message: Option<String>,
} }
record! { ActionClick: ActionClickTag = "click" {
    button: super::literals::ActionClickButtonChoice,
    #[serde(rename = "x" )]
    coordinate_x: i64,
    #[serde(rename = "y" )]
    coordinate_y: i64,
    keys: Option<Vec<String>>,
} }
record! { ActionDoubleClick: ActionDoubleClickTag = "double_click" {
    keys: Option<Vec<String>>,
    #[serde(rename = "x" )]
    coordinate_x: i64,
    #[serde(rename = "y" )]
    coordinate_y: i64,
} }
record! { ActionDrag: ActionDragTag = "drag" {
    path: Vec<ActionDragPath>,
    keys: Option<Vec<String>>,
} }
record! { ActionDragPath {
    #[serde(rename = "x" )]
    coordinate_x: i64,
    #[serde(rename = "y" )]
    coordinate_y: i64,
} }
record! { ActionKeypress: ActionKeypressTag = "keypress" {
    keys: Vec<String>,
} }
record! { ActionMove: ActionMoveTag = "move" {
    #[serde(rename = "x" )]
    coordinate_x: i64,
    #[serde(rename = "y" )]
    coordinate_y: i64,
    keys: Option<Vec<String>>,
} }
record! { ActionScreenshot: ActionScreenshotTag = "screenshot" {} }
record! { ActionScroll: ActionScrollTag = "scroll" {
    scroll_x: i64,
    scroll_y: i64,
    #[serde(rename = "x" )]
    coordinate_x: i64,
    #[serde(rename = "y" )]
    coordinate_y: i64,
    keys: Option<Vec<String>>,
} }
record! { ActionType: ActionTypeTag = "type" {
    text: String,
} }
record! { ActionWait: ActionWaitTag = "wait" {} }
record! { ComputerOutput: ComputerOutputTag = "computer_call_output" {
    id: String,
    call_id: String,
    output: ComputerOutputScreenshot,
    status: super::literals::ComputerOutputStatusChoice,
    acknowledged_safety_checks: Option<Vec<PendingSafetyCheck>>,
    created_by: Option<String>,
} }
record! { ComputerOutputScreenshot: ComputerOutputScreenshotTag = "computer_screenshot" {
    file_id: Option<String>,
    image_url: Option<String>,
} }
record! { OutputComputer: OutputComputerTag = "computer" {} }
record! { ComputerUsePreview: ComputerUsePreviewTag = "computer_use_preview" {
    display_height: i64,
    display_width: i64,
    environment: super::literals::ComputerUsePreviewEnviroChoice,
} }
