//! Brume's items on the page context menu.
//!
//! # Curated, not replaced
//!
//! `ContextMenuRequested` offers two ways out. Setting `Handled` suppresses
//! WebView2's menu entirely and leaves Brume to draw its own, which would mean
//! reimplementing positioning, keyboard navigation, submenus, and every item the
//! engine already provides for free: copy, paste, spellcheck suggestions, image
//! and media commands, inspect.
//!
//! The other way is to modify the item collection the event hands over, and let
//! WebView2 draw it. That is what this does. The menu stays native and Brume
//! only changes what needs changing.
//!
//! # What needs changing, and what did not
//!
//! Only the link items. The notes used to claim WebView2's "Open link in new
//! window" did nothing because browser.rs denies new windows. **That was wrong
//! and was never checked.** `on_new_window` spawns `open_tab_inner` before
//! returning Deny, so the item works: it opens a Brume tab. Verified by calling
//! `window.open()` from a page, the same NewWindowRequested path, which went
//! from one tab to two with the right URL activated.
//!
//! So nothing here is a repair. The item is replaced because its **label lies**:
//! it says window and delivers a tab. Brume swaps in items that say what they
//! do, and adds opening a link in a private tab, which the engine has no way to
//! offer because privacy is Brume's own idea and not the runtime's.
//!
//! # How to check this again
//!
//! A Chromium-drawn menu cannot be read back from the outside: it is not a Win32
//! menu, so there is nothing to enumerate, and driving it would need desktop
//! input this project does not use. What worked was to right-click a link over
//! CDP (`Input.dispatchMouseEvent`, contained to the webview and harmless to the
//! desktop) with stderr redirected to a file, and log what the handler did.
//!
//! Measured that way on 2026-08-05: the stock `openLinkInNewWindow` was found
//! and removed, both items were inserted at 0 and 1, and the collection went
//! from 8 items to 10. The logging is gone now; put it back the same way if this
//! ever needs checking again.
//!
//! # Custom items outlive the handler
//!
//! An item created here is dropped as soon as the menu closes, but its selected
//! handler has to survive being clicked. `CreateContextMenuItem` returns a COM
//! object whose lifetime the runtime manages once it is in the collection, so
//! the handler is registered before insertion and nothing is kept on this side.

use tauri::{AppHandle, Manager};
use webview2_com::Microsoft::Web::WebView2::Win32::{
    ICoreWebView2ContextMenuItem, ICoreWebView2Environment9, ICoreWebView2_11, ICoreWebView2_2,
    COREWEBVIEW2_CONTEXT_MENU_ITEM_KIND_COMMAND,
};
use webview2_com::{take_pwstr, ContextMenuRequestedEventHandler, CustomItemSelectedEventHandler};
use windows_core::Interface;

/// Adds Brume's items to the page context menu for one tab.
///
/// Called once per content webview, next to `history::watch`. The token is
/// dropped for the same reason the others are.
///
/// A runtime without `ICoreWebView2_11` fails the cast and simply keeps the
/// stock menu, which is a perfectly usable outcome.
pub fn watch(app: &AppHandle, label: &str) {
    let Some(webview) = app.get_webview(label) else {
        return;
    };
    let handle = app.clone();
    // A link opened from a private tab stays private, the same rule
    // `on_new_window` follows.
    let opener_private = crate::browser::tab_is_private(app, label);

    let _ = webview.with_webview(move |platform| {
        let _ = (|| unsafe {
            let core = platform.controller().CoreWebView2()?;
            let environment = core.cast::<ICoreWebView2_2>()?.Environment()?;
            let core11: ICoreWebView2_11 = core.cast()?;

            let mut token = 0i64;
            core11.add_ContextMenuRequested(
                &ContextMenuRequestedEventHandler::create(Box::new(move |_sender, args| {
                    let Some(args) = args else { return Ok(()) };
                    let target = args.ContextMenuTarget()?;

                    // Only links get Brume's items. Everything else keeps the
                    // stock menu untouched, which is the point of curating
                    // rather than replacing.
                    let mut has_link = windows_core::BOOL::from(false);
                    target.HasLinkUri(&mut has_link)?;
                    if !has_link.as_bool() {
                        return Ok(());
                    }

                    let mut raw = windows_core::PWSTR::null();
                    target.LinkUri(&mut raw)?;
                    if raw.is_null() {
                        return Ok(());
                    }
                    let link = take_pwstr(raw);
                    if link.is_empty() {
                        return Ok(());
                    }

                    let items = args.MenuItems()?;
                    let Ok(environment9) = environment.cast::<ICoreWebView2Environment9>() else {
                        eprintln!("[contextmenu] runtime has no ICoreWebView2Environment9");
                        return Ok(());
                    };

                    // Dropped first, so Brume's replacements sit where it was
                    // rather than below a duplicate that reads differently.
                    //
                    // Not asserted on: a runtime that renames or drops the item
                    // should still get Brume's two, and the worst case is one
                    // extra entry rather than a broken menu.
                    remove_named(&items, "openLinkInNewWindow");

                    let mut at = 0u32;
                    for (offset, (label, private)) in [
                        ("Open link in new tab", false),
                        ("Open link in private tab", true),
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        let item = environment9.CreateContextMenuItem(
                            &windows_core::HSTRING::from(label),
                            None,
                            COREWEBVIEW2_CONTEXT_MENU_ITEM_KIND_COMMAND,
                        )?;

                        let app = handle.clone();
                        let url = link.clone();
                        let as_private = private || opener_private;
                        let mut selected_token = 0i64;
                        item.add_CustomItemSelected(
                            &CustomItemSelectedEventHandler::create(Box::new(move |_s, _a| {
                                let app = app.clone();
                                let url = url.clone();
                                // Spawned, never inline. This runs on the main
                                // thread and open_tab_inner reaches add_child,
                                // which dispatches to the main thread and blocks
                                // on it - the deadlock browser.rs documents.
                                tauri::async_runtime::spawn(async move {
                                    // The window this menu was raised in, so a
                                    // link opened from a second window lands
                                    // there rather than in the first.
                                    let Some(w) = crate::browser::focused_window(&app) else {
                                        return;
                                    };
                                    if let Err(e) = crate::browser::open_tab(
                                        app,
                                        w,
                                        Some(url),
                                        Some(as_private),
                                    )
                                    .await
                                    {
                                        eprintln!("[contextmenu] could not open link: {e}");
                                    }
                                });
                                Ok(())
                            })),
                            &mut selected_token,
                        )?;

                        at = offset as u32;
                        items.InsertValueAtIndex(at, &item)?;
                    }
                    let _ = at;

                    Ok(())
                })),
                &mut token,
            )?;
            Ok::<_, windows_core::Error>(())
        })();
    });
}

/// Drops a stock item by its stable `Name`.
///
/// Matched on `Name` rather than `Label`: the label is localised and would stop
/// matching the moment anyone runs Brume in another language, while the name is
/// the engine's own identifier and does not move.
fn remove_named(items: &ItemCollection, name: &str) -> bool {
    let mut removed = false;
    unsafe {
        let mut count = 0u32;
        if items.Count(&mut count).is_err() {
            return false;
        }
        for i in (0..count).rev() {
            let Ok(item) = items.GetValueAtIndex(i) else {
                continue;
            };
            if item_name(&item).as_deref() == Some(name) && items.RemoveValueAtIndex(i).is_ok() {
                removed = true;
            }
        }
    }
    removed
}

type ItemCollection =
    webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2ContextMenuItemCollection;

fn item_name(item: &ICoreWebView2ContextMenuItem) -> Option<String> {
    unsafe {
        let mut raw = windows_core::PWSTR::null();
        item.Name(&mut raw).ok()?;
        if raw.is_null() {
            return None;
        }
        Some(take_pwstr(raw))
    }
}
