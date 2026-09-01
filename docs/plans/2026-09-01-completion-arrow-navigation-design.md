# Completion Arrow Navigation Design

## Goal

Allow the Up and Down arrow keys to select candidates in the SQL Editor completion popup and in the table-header `where`/`order by` candidate prompts, while preserving the existing `Ctrl+P`/`Ctrl+N` behavior.

## Behavior

- When a completion candidate list is open and contains results, Down moves the selection forward and Up moves it backward.
- Arrow navigation uses the same wrapping and selection state as `Ctrl+N` and `Ctrl+P`.
- When no completion list is active, Up and Down retain their existing editor or table navigation behavior.

## Implementation

Route `Up` and `Down` through the existing candidate-list movement branch rather than adding a second selection model. The SQL Editor and table-header prompt handlers should share the same directional mapping wherever they currently handle `Ctrl+P`/`Ctrl+N`. Add focused model or handler tests covering the equivalent forward/backward movement and the inactive-popup path where practical.

## Verification

Run the relevant Rust test targets, then run the project formatter and compiler checks used by the repository.
