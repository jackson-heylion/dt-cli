use super::callback_page;

// 页面残留占位符会把模板语法直接展示给员工。
fn assert_no_placeholder(page: &str) {
    assert!(!page.contains("{{"), "回调页残留未替换的占位符");
}

#[test]
fn accepted_page_guides_manual_close_without_promising_automatic_close() {
    let page = callback_page(true, "<script nonce=\"n\">window.close()</script>");
    assert_no_placeholder(&page);
    // 浏览器不允许网页关闭由命令行打开的标签页，只能把关闭方式写清楚。
    assert!(!page.contains("自动关闭"));
    assert!(page.contains("⌘W"));
    assert!(page.contains("window.close()"));
}

#[test]
fn rejected_page_stays_script_free_and_without_actions() {
    let page = callback_page(false, "");
    assert_no_placeholder(&page);
    assert!(!page.contains("<script"));
    assert!(!page.contains("class=\"actions\""));
    assert!(!page.contains("id=\"close-page\""));
}
