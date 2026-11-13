#!/bin/bash
set -euo pipefail
export MSSQL_SA_PASSWORD
MSSQL_SA_PASSWORD="$(cat /run/lazydb-secrets/mssql_password)"
exec /opt/mssql/bin/sqlservr
