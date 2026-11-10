#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage:
  release-mirror.sh --tag v0.1.0 --assets-dir release-assets [--dry-run]
  release-mirror.sh --latest [--dry-run]

The script downloads or reads GitHub Release assets, validates size and SHA-256,
then publishes a version directory and latest.json to the WarmDesk mirror.
USAGE
}

TAG=""
ASSETS_DIR=""
DRY_RUN=0
LATEST=0
while (($#)); do
  case "$1" in
    --tag) TAG="${2:?missing value for --tag}"; shift 2 ;;
    --assets-dir) ASSETS_DIR="${2:?missing value for --assets-dir}"; shift 2 ;;
    --latest) TAG=""; LATEST=1; shift ;;
    --dry-run) DRY_RUN=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

REPO="${GITHUB_REPOSITORY:-kokotao/warmdesk}"
NOTE_BASE_URL="${NOTE_MIRROR_BASE_URL:-https://note.lizipro.cn/warmdesk/releases}"
NOTE_ROOT="${NOTE_MIRROR_ROOT:-}"
NOTE_HOST="${NOTE_MIRROR_HOST:-}"
command -v jq >/dev/null || { echo "jq is required" >&2; exit 1; }
command -v curl >/dev/null || { echo "curl is required" >&2; exit 1; }
if [[ "$NOTE_BASE_URL" != https://* || "$NOTE_BASE_URL" == *[[:space:]]* ]]; then
  echo "NOTE_MIRROR_BASE_URL must be an HTTPS URL without whitespace" >&2
  exit 2
fi

sha256_file() {
  if command -v sha256sum >/dev/null; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

require_publish_tools() {
  command -v rsync >/dev/null || { echo "rsync is required for mirror publish" >&2; exit 1; }
  command -v ssh >/dev/null || { echo "ssh is required for mirror publish" >&2; exit 1; }
}

if ((LATEST)); then
  release_json="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest")"
  TAG="$(jq -r '.tag_name // empty' <<<"$release_json")"
fi
[[ "$TAG" =~ ^v[0-9]+\.[0-9]+\.[0-9]+([.-][A-Za-z0-9.-]+)?$ ]] || {
  echo "A valid --tag or --latest release is required" >&2
  exit 2
}
VERSION="${TAG#v}"
if [[ -z "$ASSETS_DIR" ]]; then
  mirror_json="$(curl -fsSL --max-time 20 "${NOTE_BASE_URL%/}/latest.json" 2>/dev/null || true)"
  if [[ -n "$mirror_json" ]] && jq -e --arg tag "$TAG" '.tag == $tag and (.assets | length > 0)' \
    >/dev/null 2>&1 <<<"$mirror_json"; then
    echo "MIRROR_UP_TO_DATE tag=${TAG}; skipping asset download"
    exit 0
  fi
fi
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

if [[ -z "$ASSETS_DIR" ]]; then
  ASSETS_DIR="$TMP_DIR/assets"
  mkdir -p "$ASSETS_DIR"
  if [[ -z "${release_json:-}" ]]; then
    github_api_args=(-fsSL -H "Accept: application/vnd.github+json")
    if [[ -n "${GITHUB_TOKEN:-}" ]]; then
      github_api_args+=(-H "Authorization: Bearer ${GITHUB_TOKEN}")
    fi
    release_json="$(curl "${github_api_args[@]}" \
      "https://api.github.com/repos/${REPO}/releases/tags/${TAG}")"
  fi
  release_tag="$(jq -r '.tag_name // empty' <<<"$release_json")"
  release_draft="$(jq -r 'if .draft == null then true else .draft end' <<<"$release_json")"
  release_prerelease="$(jq -r 'if .prerelease == null then true else .prerelease end' <<<"$release_json")"
  [[ "$release_tag" == "$TAG" && "$release_draft" == "false" && "$release_prerelease" == "false" ]] || {
    echo "Release must be a published, non-prerelease release matching tag ${TAG}" >&2
    exit 1
  }
  while IFS=$'\t' read -r name url expected_size expected_digest; do
    case "$name" in
      *.dmg|*.zip|*.msi|*.exe)
        [[ "$name" =~ ^[A-Za-z0-9._+-]+$ ]] || {
          echo "Unsafe release asset name: $name" >&2
          exit 1
        }
        [[ "$url" == "https://github.com/${REPO}/releases/download/${TAG}/"* ]] || {
          echo "Unexpected release asset URL for ${name}" >&2
          exit 1
        }
        target="$ASSETS_DIR/$name"
        curl -fL --connect-timeout 20 --max-time 300 \
          --retry 4 --retry-all-errors --retry-delay 2 "$url" -o "$target.part"
        actual_size="$(wc -c < "$target.part" | tr -d ' ')"
        [[ "$expected_size" == "0" || "$actual_size" == "$expected_size" ]] || {
          echo "size mismatch for $name" >&2
          exit 1
        }
        if [[ "$expected_digest" == sha256:* ]]; then
          expected_sha="${expected_digest#sha256:}"
          actual_sha="$(sha256_file "$target.part")"
          [[ "$actual_sha" == "$expected_sha" ]] || { echo "sha256 mismatch for $name" >&2; exit 1; }
        fi
        mv "$target.part" "$target"
        ;;
    esac
  done < <(jq -r '.assets[]? | [.name,.browser_download_url, (.size // 0), (.digest // "")] | @tsv' <<<"$release_json")
fi

ASSETS=()
while IFS= read -r file; do
  ASSETS+=("$file")
done < <(find "$ASSETS_DIR" -maxdepth 1 -type f \( -name '*.dmg' -o -name '*.zip' -o -name '*.msi' -o -name '*.exe' \) -print | sort)
((${#ASSETS[@]} > 0)) || { echo "No supported release assets found" >&2; exit 1; }

ASSET_JSON="$TMP_DIR/assets.json"
printf '[]\n' > "$ASSET_JSON"
for file in "${ASSETS[@]}"; do
  name="$(basename "$file")"
  size="$(wc -c < "$file" | tr -d ' ')"
  sha="$(sha256_file "$file")"
  encoded_name="$(jq -rn --arg value "$name" '$value|@uri')"
  jq --arg name "$name" --argjson size "$size" --arg sha "$sha" \
    --arg note "${NOTE_BASE_URL}/${VERSION}/${encoded_name}" \
    --arg github "https://github.com/${REPO}/releases/download/${TAG}/${encoded_name}" \
    '. + [{name:$name,size:$size,sha256:$sha,urls:{note:$note,github:$github}}]' \
    "$ASSET_JSON" > "$TMP_DIR/assets.next.json"
  mv "$TMP_DIR/assets.next.json" "$ASSET_JSON"
done
jq -n --arg version "$VERSION" --arg tag "$TAG" \
  --arg release_url "https://github.com/${REPO}/releases/tag/${TAG}" \
  --arg generated_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --slurpfile assets "$ASSET_JSON" \
  '{version:$version,tag:$tag,release_url:$release_url,generated_at:$generated_at,assets:$assets[0]}' \
  > "$TMP_DIR/latest.json"

echo "RELEASE_READY tag=${TAG} version=${VERSION} assets=${#ASSETS[@]}"
jq -r '.assets[] | "ASSET name=\(.name) bytes=\(.size) sha256=\(.sha256)"' "$TMP_DIR/latest.json"
if ((DRY_RUN)); then
  echo "DRY_RUN_COMPLETE"
  exit 0
fi
[[ -n "$NOTE_HOST" && -n "$NOTE_ROOT" ]] || {
  echo "NOTE_MIRROR_HOST and NOTE_MIRROR_ROOT are required for publish" >&2
  exit 1
}
REMOTE_VERSION="${NOTE_ROOT%/}/${VERSION}"
if [[ "$NOTE_HOST" == local || "$NOTE_HOST" == localhost ]]; then
  command -v rsync >/dev/null || { echo "rsync is required for mirror publish" >&2; exit 1; }
  mkdir -p "$REMOTE_VERSION" "${NOTE_ROOT%/}"
  rsync -avc --partial --delay-updates "${ASSETS[@]}" "${REMOTE_VERSION}/"
  while IFS=$'\t' read -r name sha; do
    remote_sha="$(sha256_file "${REMOTE_VERSION}/${name}")"
    [[ "$remote_sha" == "$sha" ]] || { echo "local mirror checksum mismatch for $name" >&2; exit 1; }
  done < <(jq -r '.assets[] | [.name,.sha256] | @tsv' "$TMP_DIR/latest.json")
  rsync -av --delay-updates "$TMP_DIR/latest.json" "${REMOTE_VERSION}/latest.json"
  rsync -av --delay-updates "$TMP_DIR/latest.json" "${NOTE_ROOT%/}/latest.json"
else
  require_publish_tools
  ssh "$NOTE_HOST" "mkdir -p '$REMOTE_VERSION' '${NOTE_ROOT%/}'"
  rsync -avc --partial --delay-updates "${ASSETS[@]}" "$NOTE_HOST:${REMOTE_VERSION}/"
  while IFS=$'\t' read -r name sha; do
    remote_sha="$(ssh "$NOTE_HOST" "if command -v sha256sum >/dev/null; then sha256sum '$REMOTE_VERSION/$name' | awk '{print \\\$1}'; else shasum -a 256 '$REMOTE_VERSION/$name' | awk '{print \\\$1}'; fi")"
    [[ "$remote_sha" == "$sha" ]] || { echo "remote checksum mismatch for $name" >&2; exit 1; }
  done < <(jq -r '.assets[] | [.name,.sha256] | @tsv' "$TMP_DIR/latest.json")
  rsync -av --delay-updates "$TMP_DIR/latest.json" "$NOTE_HOST:$REMOTE_VERSION/latest.json"
  rsync -av --delay-updates "$TMP_DIR/latest.json" "$NOTE_HOST:${NOTE_ROOT%/}/latest.json"
fi
echo "MIRROR_PUBLISHED url=${NOTE_BASE_URL}/latest.json"
