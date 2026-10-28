use super::block_detection::find_blocks;

#[test]
fn returns_blocks_in_order() {
    assert_eq!(
        find_blocks("```py\na\n``` ```js\nb\n```", None, true, 0.5).len(),
        2
    );
}
