# MongoDB

LazyDB connects with a MongoDB profile and keeps document data in BSON/EJSON form. MongoDB consoles accept one ordered JSON command object per run; the current TUI console supports bounded `find` and `count` reads. Use the Explorer to open a database, then its Monitor or Users & Roles entry. `Enter` on a collection opens its documents in the relation tab; the collection's indexes stay behind the expand keys (`Right`, `o`).

## Staged document deletion

In a collection's DATA grid, `dd` marks a document for deletion. `V` followed by
`d` marks a range. `Ctrl-s` opens the review; select Commit and press `Enter` to
submit the marked documents. Undo or discard can remove the marks before submission;
the grid waits for the result before accepting more edits, another save, or tab closure.

The write matches each document's original BSON `_id` and complete loaded
snapshot, with a limit of one deletion per marked row. An external change to a
document prevents that document from matching. Read-only profiles refuse the
write. Ordinary collections are supported without cluster administration
privileges; time-series collections, views, and capped collections are refused
before writing. Unacknowledged writes (`w=0`) are also refused. If a batch exceeds
the command size limit, select fewer documents and submit again.

A confirmed deletion invalidates the DATA page, BSON documents, and inferred
structure together. Background tabs reload when activated. Partial or uncertain
results keep the marks and ask you to discard and reload before retrying; the
bulk operation is not a transaction and may already have deleted some rows.

## Monitoring and principals

The Monitor view samples `serverStatus`, `dbStats`, and `currentOp` independently. Each request has a five second client timeout and current operations are limited to 500 displayed entries. Missing privileges are reported per command so the other metrics remain visible. Monitoring is manual; press `r` to refresh.

Users & Roles lists users, assigned and inherited roles, inherited privileges, and authentication restrictions when the server grants access. Press `u` in the Mongo monitor view to refresh principal data. Principal data is read-only in this release. LazyDB does not create or alter users and roles, accept passwords for these commands, or persist credentials in console history.

## Transactions

The console transaction controls are `Alt+B` to begin, `Alt+C` to commit, and `Alt+A` to abort. Transactions require a replica set or sharded cluster. The current transaction command allowlist is collection `find`; standalone servers reject transaction startup. An unresolved commit is shown as an unknown outcome and must be checked against the server before retrying application writes.

## Verification limits

Unit and compile checks cover command validation and partial-permission presentation. Live monitoring, user/role visibility, transaction behavior, and server permission responses require a configured MongoDB service and have not been verified in an environment without `LAZYDB_TEST_MONGODB_URL`.

Staged deletion integration tests use `LAZYDB_TEST_MONGODB_URL` with an isolated
MongoDB fixture. `LAZYDB_TEST_MONGODB_READWRITE_URL` optionally verifies deletion
with an authenticated account that has only `readWrite` on its selected database
and no cluster administration privileges.
