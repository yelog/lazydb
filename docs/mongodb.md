# MongoDB

LazyDB connects with a MongoDB profile and keeps document data in BSON/EJSON form. MongoDB consoles accept one ordered JSON command object per run; the current TUI console supports bounded `find` and `count` reads. Use the Explorer to open a database, then its Monitor or Users & Roles entry. `Enter` on a collection opens its documents in the relation tab; the collection's indexes stay behind the expand keys (`Right`, `o`).

## Monitoring and principals

The Monitor view samples `serverStatus`, `dbStats`, and `currentOp` independently. Each request has a five second client timeout and current operations are limited to 500 displayed entries. Missing privileges are reported per command so the other metrics remain visible. Monitoring is manual; press `r` to refresh.

Users & Roles lists users, assigned and inherited roles, inherited privileges, and authentication restrictions when the server grants access. Press `u` in the Mongo monitor view to refresh principal data. Principal data is read-only in this release. LazyDB does not create or alter users and roles, accept passwords for these commands, or persist credentials in console history.

## Transactions

The console transaction controls are `Alt+B` to begin, `Alt+C` to commit, and `Alt+A` to abort. Transactions require a replica set or sharded cluster. The current transaction command allowlist is collection `find`; standalone servers reject transaction startup. An unresolved commit is shown as an unknown outcome and must be checked against the server before retrying application writes.

## Verification limits

Unit and compile checks cover command validation and partial-permission presentation. Live monitoring, user/role visibility, transaction behavior, and server permission responses require a configured MongoDB service and have not been verified in an environment without `LAZYDB_TEST_MONGODB_URL`.
