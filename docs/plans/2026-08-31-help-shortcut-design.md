# Help Shortcut Design

## Goal

Make the footer's help hint display `?` instead of `F1`, document that shortcut in the contextual help panel, and make selecting the help entry and pressing Enter a no-op while the help panel is already open.

## Design

- Add a `Help` variant to `HelpShortcutId` and add a shared entry with key `? (also F1)` and description `open this help panel` to every contextual shortcut list.
- Change the footer label to `? help` so the visible hint and help documentation use the same shortcut notation.
- Keep the existing F1 action mapping as an input compatibility path; this request changes the displayed/documented shortcut without unnecessarily removing an existing way to open help.
- In `execute_help_shortcut`, handle `Help` before the generic overlay dismissal. Returning no commands leaves the current help overlay and selection unchanged.

## Verification

- Unit-test that each focus context includes the help entry and that selecting it is represented by `HelpShortcutId::Help`.
- Unit-test that executing the help entry keeps the help overlay open.
- Run `cargo fmt --check` and `cargo test`.
