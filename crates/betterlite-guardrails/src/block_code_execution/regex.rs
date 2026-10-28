use super::block_detection::find_blocks;

#[test]
fn matches_multiline_fenced_blocks() {
    let blocks = find_blocks("before\n```py\nprint('x')\n```\nafter", None, true, 0.5);
    assert_eq!(blocks.len(), 1);
}
