#!/usr/bin/env bash
set -Eeuo pipefail

LOG_DIR="${WARM_DESK_MIRROR_LOG_DIR:-/var/log/warmdesk-release-mirror}"
LOCK_FILE="${WARM_DESK_MIRROR_LOCK_FILE:-/var/cache/warmdesk-release-mirror/warmdesk.lock}"
SCRIPT="${WARM_DESK_MIRROR_SCRIPT:-/opt/warmdesk-release-mirror/release-mirror.sh}"
ENV_FILE="${WARM_DESK_MIRROR_ENV_FILE:-/etc/warmdesk/release-mirror.env}"

mkdir -p "$LOG_DIR" "$(dirname "$LOCK_FILE")"
LOG_FILE="${LOG_DIR}/mirror-$(date +%F).log"

if [[ -r "$ENV_FILE" ]]; then
  set -a
  # shellcheck disable=SC1090
  source "$ENV_FILE"
  set +a
fi

exec 9>"$LOCK_FILE"
if ! flock -n 9; then
  printf '%s [WARN] another WarmDesk mirror run is active; exiting\n' \
    "$(date --iso-8601=seconds)" | tee -a "$LOG_FILE"
  exit 0
fi

printf '%s [INFO] WarmDesk mirror run started; script=%s args=%q\n' \
  "$(date --iso-8601=seconds)" "$SCRIPT" "$*" | tee -a "$LOG_FILE"
set +e
"$SCRIPT" "$@" 2>&1 | tee -a "$LOG_FILE"
status="${PIPESTATUS[0]}"
set -e
if ((status == 0)); then
  printf '%s [INFO] WarmDesk mirror run completed successfully\n' \
    "$(date --iso-8601=seconds)" | tee -a "$LOG_FILE"
else
  printf '%s [ERROR] WarmDesk mirror run failed; exit=%s\n' \
    "$(date --iso-8601=seconds)" "$status" | tee -a "$LOG_FILE"
fi
exit "$status"
