# Omni Bar

Press `F2` from any pane or overlay to open the global action and object switcher.
The Omni Bar searches local commands, connection profiles, SQL consoles, open
tabs, and loaded catalog relations. With an active connection, non-command
queries are also sent to the database catalog after the existing debounce.

| Input | Behavior |
| --- | --- |
| Text | Search names, paths, connection labels, command titles, and aliases |
| `>` followed by text | Restrict results to commands |
| `@` followed by text | Find and select a connection scope |
| `Up` / `Down` | Change the selected result |
| `Enter` | Open the selected object or run the selected command |
| `Tab` | Show actions for a selected catalog relation |
| `Escape` | Return one step; close Omni from the root step |
| `Ctrl-C` | Dismiss Omni and restore the underlying interaction |

Opening a cached relation on another profile starts a connection switch and
continues to that exact relation only when the matching connection attempt
succeeds. A running query or an unresolved transaction may block the switch;
Omni reports that condition instead of silently interrupting database work.
Offline profiles can be selected as a local filter, but remote search requires
an active connection.

Idle Profile Manager and Catalog Editor forms are retained as in-memory
interactions when navigation proceeds. Search for “resume” or the form name to
return to one. Forms are bound to their originating profile/session and are not
written to workspace persistence. Busy operations and destructive/transaction
confirmations must be completed or cancelled before navigation.

The previous workspace location is available through the `back` / “Return to
Previous Location” command. History is process-local and bounded; it contains
workspace identities and labels, not SQL text, query text, or credentials.
