# Key Sequence Timeout Design

## Problem

Pending key sequences currently measure their 750 ms timeout from the first key. Valid continuation keys preserve the original timestamp. A counted pane resize such as `10 Ctrl-w >` can therefore expire while it is still being entered. The expired `Ctrl-w` starts a new uncounted window sequence, so the final resize uses a delta of one.

The same timing model affects every multi-stage pending sequence, not only pane width changes.

## Design

Treat the sequence timeout as the maximum delay between valid adjacent keys. Whenever a pending state accepts a key and remains pending, refresh its timestamp to the current instant. Completed commands and invalid continuation keys continue to clear pending state as before.

Keep count parsing and pane resize direction mapping unchanged. They already produce the correct counted `PaneResize` action when the state survives long enough.

## Verification

- Verify valid continuation keys refresh the pending timestamp.
- Verify a counted window command preserves a multi-digit count across refreshed transitions.
- Verify stale sequences still expire.
- Verify uncounted and counted width and height resize mappings remain correct.
