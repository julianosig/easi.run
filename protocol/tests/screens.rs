//! The screen sizes the admin page starts sessions at (client/src/ui/screens.ts) all follow the output-size rule: a
//! size SVT-AV1 crashes on would crash the session's display as its first viewer connects.

use display_protocol::frame::output_size_ok;

#[test]
fn every_screen_the_admin_page_picks_is_an_output_size_the_display_encodes() {
    // Included, so a change to the list rebuilds this test, and the checks a change selects find it.
    let source = include_str!("../../client/src/ui/screens.ts");
    let list = &source[source.find("SCREENS").unwrap()..];
    let list = &list[..list.find("];").unwrap()];
    let sizes: Vec<(u32, u32)> = list
        .split('[')
        .filter_map(|item| {
            let (w, h) = item.split(']').next()?.split_once(',')?;
            Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
        })
        .collect();
    assert!(sizes.len() >= 8, "the screens read: {sizes:?}");
    for (w, h) in sizes {
        assert!(output_size_ok(w, h), "{w}x{h}");
    }
}
