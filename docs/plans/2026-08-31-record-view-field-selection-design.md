# Record View Field Selection Design

## Goal

Make field navigation visible and useful even when every field fits in the
Record View viewport.

## State And Interaction

`RecordViewState` stores both the selected field index and the viewport offset.
The selected field is the interaction state; the offset only controls which
fields are visible.

- Record View initially selects the first field.
- `j/k` and Down/Up move the selection by one field and clamp at the bounds.
- `gg` and Home select the first field.
- `G` and End select the last field.
- Moving outside the visible range scrolls just enough to reveal the selection.
- Changing records resets selection and offset to the first field.
- Replacing the result with fewer fields clamps both values safely.

## Presentation

Render the selected field as a full-width highlighted row using the existing
theme selection background. Field, type, and value text must remain readable on
that background. When all fields fit, the offset stays at zero while the
highlight still moves.

## Testing

- Model tests cover movement, bounds, first/last jumps, viewport following, and
  clamping after field-count changes.
- App tests cover resetting selection when moving between records.
- UI tests verify that the selected row receives the selection styling and that
  moving the selection changes the highlighted row.
