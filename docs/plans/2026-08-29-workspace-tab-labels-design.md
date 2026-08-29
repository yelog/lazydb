# Workspace Tab Labels Design

## Goal

Replace positional workspace-tab prefixes with icons that communicate each
tab's content type. Relation previews display their catalog object icon and SQL
editors display their database-driver icon.

## Design

Keep tab identity, ordering, names, and persistence unchanged. The numeric
prefix is currently added only by `render_tabs`, so the new label remains a UI
projection with the format ` icon title `.

Relation tabs use `RelationDescriptor.kind` with the existing
`IconSet::catalog` mapping. SQL tabs resolve their icon from the profile named by
`execution_target.profile_id`; if that target is unavailable, they fall back to
the active profile, then the connected server kind, and finally the generic
database catalog icon. This preserves useful context for new or orphaned SQL
editors without adding display state to `WorkspaceTab`.

The existing title sanitization, 48-character limit, active/inactive styling,
and mouse hit regions remain unchanged. All icon modes continue to flow through
the existing `IconSet`, so Nerd Font, Unicode, and ASCII rendering stay
consistent with the Explorer.

## Testing

Use ASCII icons for deterministic render assertions. Cover SQL and relation
labels, absence of numeric prefixes, and the rule that a SQL tab's bound profile
takes precedence over the currently active connection.
