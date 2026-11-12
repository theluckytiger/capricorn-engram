//! Windows UI Automation: the real controls of a window (buttons, fields, links, tabs, toggles…), and pressing them
//! the way the app itself expects (invoke, toggle, select, expand), with a mouse click only as the last resort.

use anyhow::{Result, bail};
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::*;

use super::com;
use crate::platform::Control;

fn automation() -> Result<IUIAutomation> {
    com();
    Ok(unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)? })
}

/// Words for a control type, as people say them.
#[allow(non_upper_case_globals)] // windows-rs names these constants in CamelCase
fn kind_name(id: UIA_CONTROLTYPE_ID) -> &'static str {
    match id {
        UIA_ButtonControlTypeId => "button",
        UIA_CheckBoxControlTypeId => "checkbox",
        UIA_ComboBoxControlTypeId => "dropdown",
        UIA_EditControlTypeId => "field",
        UIA_HyperlinkControlTypeId => "link",
        UIA_ImageControlTypeId => "image",
        UIA_ListItemControlTypeId => "list item",
        UIA_ListControlTypeId => "list",
        UIA_MenuItemControlTypeId => "menu item",
        UIA_MenuControlTypeId => "menu",
        UIA_RadioButtonControlTypeId => "radio button",
        UIA_SliderControlTypeId => "slider",
        UIA_TabItemControlTypeId => "tab",
        UIA_TextControlTypeId => "text",
        UIA_TreeItemControlTypeId => "tree item",
        UIA_DocumentControlTypeId => "document",
        UIA_SplitButtonControlTypeId => "split button",
        UIA_DataItemControlTypeId => "item",
        UIA_GroupControlTypeId => "group",
        UIA_HeaderItemControlTypeId => "header",
        UIA_ToolBarControlTypeId => "toolbar",
        UIA_WindowControlTypeId => "window",
        UIA_PaneControlTypeId => "pane",
        UIA_CustomControlTypeId => "custom",
        _ => "other",
    }
}

/// The visible, named (or actionable) controls of a window, in tree order, at most `max`. Chromium and Electron apps
/// (browsers, Discord, Spotify…) show only their pane until something asks the way a screen reader does: their render
/// windows are asked for their UI Automation root, and the window is read again once the tree has been built (the
/// first time takes up to a couple of seconds; after that it stays on).
pub fn controls(window: isize, max: usize) -> Result<Vec<Control>> {
    let first = read_controls(window, max)?;
    let renderers = chromium_renderers(HWND(window as *mut _));
    // A Chromium window with a render window is woken once (a browser's toolbar reads fine without it, its page
    // doesn't). One that shows almost nothing is read again a moment later: the first read itself is often what
    // switches its tree on (Discord: 1 control, then 155).
    let fresh = !renderers.is_empty() && WOKEN.lock().unwrap().insert(window);
    if !fresh && first.len() > 3 {
        return Ok(first);
    }
    for r in &renderers {
        wake_accessibility(*r);
    }
    let mut best = first;
    for wait in [400, 900, 1500] {
        std::thread::sleep(std::time::Duration::from_millis(wait));
        let again = read_controls(window, max)?;
        let grew = again.len() > best.len() + 3;
        if again.len() > best.len() {
            best = again;
        }
        // Once it has grown and stopped growing, it's built.
        if !grew && best.len() > 3 {
            break;
        }
    }
    Ok(best)
}

/// Chromium windows already woken, by handle.
static WOKEN: std::sync::Mutex<std::collections::BTreeSet<isize>> = std::sync::Mutex::new(std::collections::BTreeSet::new());

/// Chromium's render windows inside a window ("Chrome_RenderWidgetHostHWND").
fn chromium_renderers(top: HWND) -> Vec<HWND> {
    use windows::Win32::Foundation::LPARAM;
    use windows::Win32::UI::WindowsAndMessaging::{EnumChildWindows, GetClassNameW};
    use windows::core::BOOL;
    let mut found: Vec<HWND> = Vec::new();
    unsafe extern "system" fn each(hwnd: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: lparam is the &mut Vec passed below, alive for the whole EnumChildWindows call.
        let found = unsafe { &mut *(lparam.0 as *mut Vec<HWND>) };
        let mut buf = [0u16; 64];
        let n = unsafe { GetClassNameW(hwnd, &mut buf) };
        if String::from_utf16_lossy(&buf[..n as usize]) == "Chrome_RenderWidgetHostHWND" {
            found.push(hwnd);
        }
        BOOL(1)
    }
    unsafe {
        let _ = EnumChildWindows(Some(top), Some(each), LPARAM(&mut found as *mut _ as isize));
    }
    found
}

/// What a screen reader sends: WM_GETOBJECT for the UI Automation root. Chromium turns its accessibility on for it.
fn wake_accessibility(hwnd: HWND) {
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_GETOBJECT};
    const UIA_ROOT_OBJECT_ID: isize = -25;
    unsafe {
        let mut result = 0usize;
        let _ = SendMessageTimeoutW(hwnd, WM_GETOBJECT, WPARAM(0), LPARAM(UIA_ROOT_OBJECT_ID), SMTO_ABORTIFHUNG, 500, Some(&mut result));
    }
}

fn read_controls(window: isize, max: usize) -> Result<Vec<Control>> {
    let uia = automation()?;
    unsafe {
        let root = uia.ElementFromHandle(HWND(window as *mut _))?;
        let cache = uia.CreateCacheRequest()?;
        for p in [
            UIA_NamePropertyId,
            UIA_ControlTypePropertyId,
            UIA_BoundingRectanglePropertyId,
            UIA_IsEnabledPropertyId,
            UIA_IsOffscreenPropertyId,
            UIA_AutomationIdPropertyId,
            UIA_ToggleToggleStatePropertyId,
            UIA_IsTogglePatternAvailablePropertyId,
            UIA_IsInvokePatternAvailablePropertyId,
            UIA_HasKeyboardFocusPropertyId,
        ] {
            cache.AddProperty(p)?;
        }
        let control_view = uia.ControlViewCondition()?;
        let on_screen = uia.CreatePropertyCondition(UIA_IsOffscreenPropertyId, &VARIANT::from(false))?;
        let cond = uia.CreateAndCondition(&control_view, &on_screen)?;
        let found = root.FindAllBuildCache(TreeScope_Descendants, &cond, &cache)?;
        let n = found.Length()?;
        let mut out = Vec::new();
        for i in 0..n {
            let e = found.GetElement(i)?;
            let name = e.CachedName().map(|b| b.to_string()).unwrap_or_default();
            let ty = e.CachedControlType().unwrap_or(UIA_CustomControlTypeId);
            let kind = kind_name(ty);
            let invokable = bool_prop(&e, UIA_IsInvokePatternAvailablePropertyId);
            let togglable = bool_prop(&e, UIA_IsTogglePatternAvailablePropertyId);
            let actionable = invokable || togglable || matches!(kind, "field" | "link" | "tab" | "list item" | "menu item" | "dropdown" | "checkbox" | "radio button" | "slider");
            if name.trim().is_empty() && !actionable {
                continue;
            }
            if matches!(kind, "pane" | "group" | "custom" | "other") && name.trim().is_empty() {
                continue;
            }
            let r: RECT = e.CachedBoundingRectangle().unwrap_or_default();
            if r.right <= r.left || r.bottom <= r.top {
                continue;
            }
            let toggled = if togglable {
                e.GetCachedPropertyValue(UIA_ToggleToggleStatePropertyId).ok().and_then(|v| i32::try_from(&v).ok()).map(|s| s == ToggleState_On.0)
            } else {
                None
            };
            out.push(Control {
                index: out.len(),
                name: name.trim().to_string(),
                kind: kind.to_string(),
                rect: [r.left, r.top, r.right, r.bottom],
                enabled: e.CachedIsEnabled().map(|b| b.as_bool()).unwrap_or(true),
                toggled,
                focused: bool_prop(&e, UIA_HasKeyboardFocusPropertyId),
                id: e.CachedAutomationId().map(|b| b.to_string()).unwrap_or_default(),
            });
            if out.len() >= max {
                break;
            }
        }
        Ok(out)
    }
}

fn bool_prop(e: &IUIAutomationElement, prop: UIA_PROPERTY_ID) -> bool {
    unsafe { e.GetCachedPropertyValue(prop).ok().and_then(|v| bool::try_from(&v).ok()).unwrap_or(false) }
}

/// Finds the live element again (by its place and name) to act on it.
fn element_at(uia: &IUIAutomation, window: isize, c: &Control) -> Result<IUIAutomationElement> {
    unsafe {
        let root = uia.ElementFromHandle(HWND(window as *mut _))?;
        let cond = uia.CreatePropertyCondition(UIA_NamePropertyId, &VARIANT::from(windows::core::BSTR::from(c.name.as_str())))?;
        let all = root.FindAll(TreeScope_Descendants, &cond)?;
        for i in 0..all.Length()? {
            let e = all.GetElement(i)?;
            let r = e.CurrentBoundingRectangle()?;
            if [r.left, r.top, r.right, r.bottom] == c.rect {
                return Ok(e);
            }
        }
        // Moved or renamed since it was read: the element under its centre, if that's still it.
        let (x, y) = c.center();
        let e = uia.ElementFromPoint(POINT { x, y })?;
        Ok(e)
    }
}

/// How the press was done, for her report.
#[derive(Debug)]
pub enum Pressed {
    Invoked,
    Toggled(bool),
    Selected,
    Expanded,
    Focused,
    Clicked,
}

/// Presses a control the app's own way; a real mouse click on its centre only when nothing else works.
pub fn press_control(window: isize, c: &Control) -> Result<Pressed> {
    let uia = automation()?;
    let e = element_at(&uia, window, c)?;
    unsafe {
        if let Ok(p) = e.GetCurrentPatternAs::<IUIAutomationTogglePattern>(UIA_TogglePatternId) {
            let before = p.CurrentToggleState()?;
            p.Toggle()?;
            // Web apps (Discord) update the switch a moment after the press, sometimes as a new element: wait for the
            // state to change (up to ~1 s), finding the control again if this one went stale. Reading it at once
            // reported a working mute as failed.
            let mut now = p.CurrentToggleState().unwrap_or(before);
            for i in 0..10 {
                if now != before {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
                now = p.CurrentToggleState().unwrap_or(before);
                if now == before
                    && i >= 3
                    && let Ok(fresh) = element_at(&uia, window, c)
                    && let Ok(fp) = fresh.GetCurrentPatternAs::<IUIAutomationTogglePattern>(UIA_TogglePatternId)
                {
                    now = fp.CurrentToggleState().unwrap_or(before);
                }
            }
            return Ok(Pressed::Toggled(now == ToggleState_On));
        }
        if let Ok(p) = e.GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId) {
            p.Invoke()?;
            return Ok(Pressed::Invoked);
        }
        if let Ok(p) = e.GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(UIA_SelectionItemPatternId) {
            p.Select()?;
            return Ok(Pressed::Selected);
        }
        if let Ok(p) = e.GetCurrentPatternAs::<IUIAutomationExpandCollapsePattern>(UIA_ExpandCollapsePatternId) {
            p.Expand()?;
            return Ok(Pressed::Expanded);
        }
        if c.kind == "field" {
            e.SetFocus()?;
            return Ok(Pressed::Focused);
        }
    }
    click_at(&uia, c)?;
    Ok(Pressed::Clicked)
}

/// A real left click on the control's centre, after checking it's still what's under that point.
fn click_at(uia: &IUIAutomation, c: &Control) -> Result<()> {
    let (x, y) = c.center();
    unsafe {
        let under = uia.ElementFromPoint(POINT { x, y })?;
        let r = under.CurrentBoundingRectangle()?;
        let inside = r.left >= c.rect[0] - 2 && r.top >= c.rect[1] - 2 && r.right <= c.rect[2] + 2 && r.bottom <= c.rect[3] + 2;
        let same = under.CurrentName().map(|n| n == c.name).unwrap_or(false);
        // Her own overlay is over every point (it's on top of everything): that isn't something covering it.
        let hers = under.CurrentProcessId().is_ok_and(|p| p as u32 == std::process::id());
        if !inside && !same && !hers {
            bail!("something else is over \"{}\" now (it moved, or another window covers it)", c.name);
        }
    }
    super::screen::click_point(x, y)
}

/// Puts text in a field through its Value pattern (replacing what's there); `false` if the field doesn't support it.
pub fn set_value(window: isize, c: &Control, text: &str) -> Result<bool> {
    let uia = automation()?;
    let e = element_at(&uia, window, c)?;
    unsafe {
        // A slider takes its number first (its text value took "\"80\"" and the page put it at half).
        if e.CurrentControlType()? == UIA_SliderControlTypeId
            && let Ok(n) = text.trim().trim_matches(['"', '\'', '“', '”']).trim().trim_end_matches('%').trim().parse::<f64>()
            && let Ok(p) = e.GetCurrentPatternAs::<IUIAutomationRangeValuePattern>(UIA_RangeValuePatternId)
            && !p.CurrentIsReadOnly()?.as_bool()
        {
            let (lo, hi) = (p.CurrentMinimum()?, p.CurrentMaximum()?);
            p.SetValue(n.clamp(lo.min(hi), hi.max(lo)))?;
            return Ok(true);
        }
        // A dropdown: its option chosen as a person does, before anything else (a web page's dropdown also takes a
        // text value, and setting that changed nothing the page saw: she said "chose Large" and it stayed Medium).
        // One you type in (an editable box with suggestions) gets the text value if choosing found nothing.
        let dropdown = e.CurrentControlType()? == UIA_ComboBoxControlTypeId;
        if dropdown && pick_option(&uia, &e, c, text)? {
            return Ok(true);
        }
        match e.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId) {
            Ok(p) if !p.CurrentIsReadOnly()?.as_bool() && !dropdown => {
                p.SetValue(&windows::core::BSTR::from(text))?;
                Ok(true)
            }
            Ok(p) if !p.CurrentIsReadOnly()?.as_bool() => {
                // An editable dropdown: what it shows afterwards says whether it took it.
                p.SetValue(&windows::core::BSTR::from(text))?;
                Ok(p.CurrentValue().is_ok_and(|v| v.to_string().trim().eq_ignore_ascii_case(text.trim())) && pick_option(&uia, &e, c, text).unwrap_or(false))
            }
            _ => {
                if dropdown {
                    return Ok(false);
                }
                // A slider: its number, kept within its range.
                let Ok(n) = text.trim().trim_end_matches('%').trim().parse::<f64>() else { return Ok(false) };
                match e.GetCurrentPatternAs::<IUIAutomationRangeValuePattern>(UIA_RangeValuePatternId) {
                    Ok(p) if !p.CurrentIsReadOnly()?.as_bool() => {
                        let (lo, hi) = (p.CurrentMinimum()?, p.CurrentMaximum()?);
                        p.SetValue(n.clamp(lo.min(hi), hi.max(lo)))?;
                        Ok(true)
                    }
                    _ => Ok(false),
                }
            }
        }
    }
}

/// Chooses the option called `name` (any case) in dropdown `e` as a person does: opened, the option's name typed,
/// Enter. (Choosing it through UI Automation changed nothing a web page could see, and focusing it that way didn't
/// give it the keyboard.) True only when the dropdown then shows that option; when it can't be read, its answer.
fn pick_option(uia: &IUIAutomation, e: &IUIAutomationElement, c: &Control, name: &str) -> Result<bool> {
    let want = name.trim();
    let value = |e: &IUIAutomationElement| unsafe { e.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId).ok().and_then(|p| p.CurrentValue().ok()).map(|v| v.to_string()) };
    let shows = |e: &IUIAutomationElement| value(e).is_some_and(|v| v.trim().eq_ignore_ascii_case(want));
    unsafe {
        let expand = e.GetCurrentPatternAs::<IUIAutomationExpandCollapsePattern>(UIA_ExpandCollapsePatternId).ok();
        // Chosen means shut again and showing it: while its list is open, it shows whichever option is highlighted.
        let closed = || expand.as_ref().is_none_or(|x| x.CurrentExpandCollapseState().is_ok_and(|s| s != ExpandCollapseState_Expanded));
        if shows(e) && closed() {
            return Ok(true);
        }
        // Opened with a real click (the list then has the keyboard), the option's name typed, Enter.
        if click_at(uia, c).is_ok() {
            std::thread::sleep(std::time::Duration::from_millis(400));
            if super::type_text(want).is_ok() {
                std::thread::sleep(std::time::Duration::from_millis(250));
                let _ = super::press("enter");
                std::thread::sleep(std::time::Duration::from_millis(400));
                if shows(e) && closed() {
                    return Ok(true);
                }
            }
        }
        // Its options in the tree: the one by that name selected (an app's own dropdown may take this).
        let cond = uia.CreatePropertyConditionEx(UIA_NamePropertyId, &VARIANT::from(windows::core::BSTR::from(want)), PropertyConditionFlags_IgnoreCase)?;
        let option = e.FindFirst(TreeScope_Descendants, &cond).ok();
        let chosen = option.is_some_and(|o| match o.GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(UIA_SelectionItemPatternId) {
            Ok(p) => p.Select().is_ok(),
            Err(_) => o.GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId).is_ok_and(|p| p.Invoke().is_ok()),
        });
        if let Some(x) = &expand
            && x.CurrentExpandCollapseState().is_ok_and(|s| s == ExpandCollapseState_Expanded)
        {
            let _ = x.Collapse();
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
        Ok(match value(e) {
            Some(v) => v.trim().eq_ignore_ascii_case(want),
            None => chosen,
        })
    }
}

/// Whether the keyboard's focus is somewhere that takes typing (a text box, an editable field), as far as UI Automation
/// can tell. None when it can't.
pub fn focus_takes_text() -> Option<bool> {
    let uia = automation().ok()?;
    unsafe {
        let e = uia.GetFocusedElement().ok()?;
        let kind = e.CurrentControlType().ok()?;
        if kind == UIA_EditControlTypeId || kind == UIA_ComboBoxControlTypeId {
            return Some(true);
        }
        if let Ok(p) = e.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
            && !p.CurrentIsReadOnly().map(|b| b.as_bool()).unwrap_or(true)
        {
            return Some(true);
        }
        // A page or a pane with the focus: typing goes nowhere.
        Some(!(kind == UIA_DocumentControlTypeId || kind == UIA_PaneControlTypeId || kind == UIA_GroupControlTypeId))
    }
}

/// A control scrolled out of sight (item 60 of a long list), found by its name (any case) and scrolled into view.
/// True when it was found and scrolled to.
pub fn scroll_into_view(window: isize, name: &str) -> Result<bool> {
    let uia = automation()?;
    unsafe {
        let root = uia.ElementFromHandle(HWND(window as *mut _))?;
        let cond = uia.CreatePropertyConditionEx(UIA_NamePropertyId, &VARIANT::from(windows::core::BSTR::from(name.trim())), PropertyConditionFlags_IgnoreCase)?;
        let Ok(e) = root.FindFirst(TreeScope_Descendants, &cond) else { return Ok(false) };
        match e.GetCurrentPatternAs::<IUIAutomationScrollItemPattern>(UIA_ScrollItemPatternId) {
            Ok(p) => {
                p.ScrollIntoView()?;
                std::thread::sleep(std::time::Duration::from_millis(350));
                Ok(true)
            }
            Err(_) => Ok(false),
        }
    }
}

/// What UI Automation says about a control, for checking one by hand: its value, whether it opens, its options.
pub fn inspect(window: isize, c: &Control) -> Result<String> {
    let uia = automation()?;
    let e = element_at(&uia, window, c)?;
    unsafe {
        let value = e.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId).ok().and_then(|p| p.CurrentValue().ok()).map(|v| v.to_string());
        let opens = e.GetCurrentPatternAs::<IUIAutomationExpandCollapsePattern>(UIA_ExpandCollapsePatternId).ok().and_then(|p| p.CurrentExpandCollapseState().ok()).map(|s| s.0);
        let selection = e.GetCurrentPatternAs::<IUIAutomationSelectionPattern>(UIA_SelectionPatternId).is_ok();
        let kids = e.FindAll(TreeScope_Descendants, &uia.CreateTrueCondition()?)?;
        let mut names = Vec::new();
        for i in 0..kids.Length()?.min(12) {
            names.push(kids.GetElement(i)?.CurrentName().map(|n| n.to_string()).unwrap_or_default());
        }
        Ok(format!("value {value:?}, expand state {opens:?}, selection pattern {selection}, descendants {names:?}, type {}", e.CurrentControlType()?.0))
    }
}
