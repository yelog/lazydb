# Completion Popup Interaction Design

## Goal

Make automatic SQL completion cursor-anchored and non-modal while prioritizing
SQL keywords in general statement contexts.

## Rendering

`render_editor` remains the only source of editor geometry. It returns the
absolute cursor cell and text viewport derived from `EditorRenderSnapshot`.
Completion renders below the cursor when space permits, otherwise above it, and
clamps width and height to the editor text viewport. No cursor coordinates are
stored in `CompletionPopup`, so resize and scroll cannot make an anchor stale.

## Ranking

Keywords always participate when completion is not qualifier-specific. In a
general SQL context, matching keywords rank above catalog objects. Relation,
qualifier, and routine contexts continue to prefer their semantic catalog kinds.
Stable lexical ordering remains the final tie-breaker.

## Input

An open popup intercepts only `Ctrl-N`, `Ctrl-P`, Enter, and Escape. Other keys
continue through normal key mapping. Before an editor key or paste is applied,
App dismisses the old popup; the resulting edit schedules or immediately
triggers a fresh completion. This prevents stale candidates without swallowing
typed characters.

## Verification

Tests cover keyword-first ranking for `s`, typing through an open popup, popup
dismissal and refresh behavior, cursor-relative placement, upward fallback, and
editor-bound clamping.
