#!/usr/bin/env bash
# check-api-changelogs.sh
#
# Checks provider changelog/release-notes pages for API-breaking keywords
# that are relevant to purple's specific API usage per provider.
# Designed to run daily via GitHub Actions (.github/workflows/api-changelog.yml).
#
# Usage:
#   ./scripts/check-api-changelogs.sh           # print new matches to stdout
#   ./scripts/check-api-changelogs.sh --update   # also update the state file
#
# Exit codes:
#   0  no new matches
#   1  new matches found (printed to stdout)
#   2  usage/dependency error

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
STATE_FILE="$REPO_ROOT/tests/api_contracts/.changelog-state.json"

# Temp directory for per-provider hash files. Cleaned up on exit.
WORK_DIR="$(mktemp -d)"
trap 'rm -rf "$WORK_DIR"' EXIT

UPDATE=false
if [[ "${1:-}" == "--update" ]]; then
    UPDATE=true
fi

# --- dependencies -----------------------------------------------------------

for cmd in curl jq; do
    if ! command -v "$cmd" &>/dev/null; then
        echo "error: $cmd is required but not found" >&2
        exit 2
    fi
done

# macOS ships shasum instead of sha256sum
if ! command -v sha256sum &>/dev/null; then
    if command -v shasum &>/dev/null; then
        sha256sum() { shasum -a 256 "$@"; }
    else
        echo "error: sha256sum (or shasum) is required but not found" >&2
        exit 2
    fi
fi

# --- keywords ---------------------------------------------------------------

KEYWORDS='deprecated|deprecate|sunset|breaking|removed|end[- ]of[- ]life|eol|discontinued|retire[ds]?'

# --- per-provider service keywords ------------------------------------------
# Two-stage filter: first match on breaking keywords, then require a service
# keyword that's relevant to the API purple actually uses. This eliminates
# noise from unrelated services (e.g. Azure ML retirements, doctl CLI changes).
# Keep these in sync with docs/providers-api.md.

declare -A SERVICE_KEYWORDS=(
    [aws]="ec2|describeinstances|describeimages|instance|ami"
    # Azure Compute has many sub-services (AKS, Batch, VM Scale Sets). purple
    # only touches virtual machines, network interfaces and public IPs. Match
    # on specific nouns, not the broad "compute" or "vm" token, so entries
    # like "Azure Kubernetes Service" under Compute do not trigger alerts.
    [azure]="virtual.?machine|network.?interface|public.?ip|microsoft\.compute/virtualmachines|microsoft\.network/networkinterfaces|microsoft\.network/publicipaddresses"
    [digitalocean]="droplet|/v2/droplet"
    [gcp]="compute.engine|instances|aggregated.?list|compute/v1"
    [hetzner]="server|/v1/server|datacenter|location|image"
    [netbox]="dcim|virtualization|/api/dcim|/api/virtualization|rest.api|serializer|device"
    [oracle]="compute|instance|vnic|compartment|core/|iaas"
    [ovhcloud]="public.cloud|instance|/cloud/project"
    [proxmox]="qemu|lxc|cluster.resource|guest.agent|api2"
    [scaleway]="instance|/instance/v1|server"
    [tailscale]="device|/api/v2/device|tailnet"
    [teleport]="\\btsh\\b|tsh ls|tsh config|tsh status|proxy ssh|openssh|ssh_config|--format=json"
    [transip]="vps|/v6/vps"
)

# --- per-provider exclusion keywords ----------------------------------------
# Stage 3: drop lines that mention services/products purple does NOT call.
# Only used when the line does NOT also contain a strong API-path token
# (see STRONG_API_TOKENS below). Keep conservative: every entry must be a
# service purple has no provider code for. Verify against docs/providers-api.md
# before adding.

declare -A EXCLUDE_KEYWORDS=(
    # Azure: Batch, AKS/Kubernetes, Scale Sets, VMware Solution, Arc, Stack,
    # HPC Cache, Machine Learning, Windows 365, Dev Box, Container Apps, Spring
    # Apps, App Service, Functions, Synapse, HDInsight.
    [azure]="batch|kubernetes|\\baks\\b|scale.?set|vmware|\\barc\\b|\\bstack\\b|\\bhpc\\b|machine.learning|windows.365|dev.?box|container.app|spring.app|app.?service|\\bfunctions\\b|synapse|hdinsight"
    [aws]="lambda|fargate|eks|ecs|lightsail|outposts|batch|beanstalk|sagemaker|workspaces"
    [gcp]="kubernetes|\\bgke\\b|cloud.?run|cloud.?functions|app.?engine|dataproc|dataflow"
    [oracle]="kubernetes|\\boke\\b|functions|autonomous|exadata|\\bdb.system\\b"
)

# Strong API-path tokens that override exclusion. If a line mentions both an
# excluded service AND a path token purple actually calls, keep the match
# (defensive: rare but possible, e.g. "Batch now uses the
# Microsoft.Compute/virtualMachines API differently").
declare -A STRONG_API_TOKENS=(
    [azure]="microsoft\\.compute/virtualmachines|microsoft\\.network/networkinterfaces|microsoft\\.network/publicipaddresses"
    [aws]="describeinstances|describeimages|ec2\\.amazonaws"
    [gcp]="compute/v1/projects|aggregatedlist"
    [oracle]="core/20160918|/instances\\?|/vnicattachments"
)

# --- provider feeds ---------------------------------------------------------
# Format: "provider|type|url"
# type: rss    = RSS/Atom XML
#       html   = raw HTML page
#       md     = raw Markdown (GitHub raw URLs)
#       ghdir  = GitHub API directory listing (fetches raw .md/.mdx files)
#       mwapi  = MediaWiki API (returns wikitext as JSON)
#
# Providers without curl-accessible changelogs (JS-rendered SPAs with no
# alternative source). Covered by Phase 3 (OpenAPI schema validation) instead:
#   Leaseweb  — Tier B, JS SPA, has vendored OpenAPI spec
#   i3D.net   — Tier B, JS SPA, no published spec (docs-only validation)
#   UpCloud   — Tier B, JS SPA, has vendored OpenAPI spec
#   Linode    — JS SPA, GitHub releases empty, OpenAPI spec has 154 deprecated markers
#   Vultr     — Returns 403 to curl; no RSS or raw-markdown source available

FEEDS=(
    "aws|rss|https://docs.aws.amazon.com/AWSEC2/latest/UserGuide/amazon-ec2-release-notes.rss"
    "azure|rss|https://www.microsoft.com/releasecommunications/api/v2/azure/rss"
    "digitalocean|html|https://docs.digitalocean.com/release-notes/api/"
    "gcp|rss|https://cloud.google.com/feeds/compute-release-notes.xml"
    "hetzner|rss|https://docs.hetzner.cloud/changelog/feed.rss"
    "netbox|rss|https://github.com/netbox-community/netbox/releases.atom"
    "oracle|rss|https://docs.oracle.com/en-us/iaas/releasenotes/feed/"
    "ovhcloud|md|https://raw.githubusercontent.com/ovh/docs/develop/pages/public_cloud/compute/image-life-cycle/guide.en-gb.md"
    "proxmox|mwapi|https://pve.proxmox.com/mediawiki/api.php?action=parse&page=Roadmap&prop=wikitext&format=json"
    "scaleway|ghdir|https://api.github.com/repos/scaleway/docs-content/contents/changelog"
    "tailscale|rss|https://tailscale.com/changelog/index.xml"
    "teleport|rss|https://github.com/gravitational/teleport/releases.atom"
    "transip|html|https://api.transip.nl/rest/docs.html"
)

# --- state management -------------------------------------------------------

init_state() {
    if [[ ! -f "$STATE_FILE" ]]; then
        echo '{"_version":1}' > "$STATE_FILE"
    elif ! jq -e '._version' "$STATE_FILE" &>/dev/null; then
        # Migrate: add version to existing state file
        local tmp
        tmp="$(mktemp "$STATE_FILE.XXXXXX")"
        jq '. + {"_version": 1}' "$STATE_FILE" > "$tmp"
        mv "$tmp" "$STATE_FILE"
    fi
}

init_state

get_seen() {
    local provider="$1"
    jq -r --arg p "$provider" '.[$p] // empty | .[]' "$STATE_FILE" 2>/dev/null
}

# --- fetch and scan ---------------------------------------------------------

FETCH_FAILURES=0
FETCH_FAILURE_LIST=""

hash_line() {
    echo -n "$1" | sha256sum | cut -d' ' -f1
}

fetch_content() {
    local url="$1"
    # Use a browser-like User-Agent so sites with bot detection (e.g. Vultr)
    # do not serve a stub page that looks like a JS-SPA. Capture HTTP status
    # so geo-blocks and rate limits surface as explicit errors instead of
    # being mistaken for JS-rendered pages.
    local tmp http_code
    tmp="$(mktemp)"
    http_code="$(
        curl -sL --max-time 30 --retry 2 --retry-delay 5 \
            -H "User-Agent: Mozilla/5.0 (compatible; purple-changelog-monitor/1.0; +https://github.com/erickochen/purple)" \
            -H "Accept: text/html,application/xhtml+xml,application/xml,application/json,text/plain,*/*" \
            -w "%{http_code}" \
            -o "$tmp" \
            "$url" 2>/dev/null || echo "000"
    )"
    if [[ "$http_code" != "200" ]]; then
        echo "  http $http_code for $url" >&2
        rm -f "$tmp"
        return
    fi
    cat "$tmp"
    rm -f "$tmp"
}

# Fetch Scaleway changelogs from GitHub API directory listing.
# Lists the 3 most recent month directories, fetches raw .mdx files from each.
fetch_scaleway_ghdir() {
    local base_url="$1"
    local dirs
    dirs="$(fetch_content "$base_url")"
    if [[ -z "$dirs" ]]; then
        return
    fi

    # Get the 3 most recent month directories (sorted alphabetically, last = newest)
    local recent_dirs
    recent_dirs="$(echo "$dirs" | jq -r '[.[] | select(.type == "dir")] | sort_by(.name) | reverse | .[0:3] | .[].url')"

    local all_content=""
    while IFS= read -r dir_url; do
        [[ -z "$dir_url" ]] && continue
        local files
        files="$(fetch_content "$dir_url")"
        [[ -z "$files" ]] && continue

        # Fetch raw content of each .mdx file
        local download_urls
        download_urls="$(echo "$files" | jq -r '.[] | select(.name | endswith(".mdx")) | .download_url')"
        while IFS= read -r file_url; do
            [[ -z "$file_url" ]] && continue
            local file_content
            file_content="$(fetch_content "$file_url")"
            if [[ -n "$file_content" ]]; then
                all_content+="$file_content"$'\n'
            fi
        done <<< "$download_urls"
    done <<< "$recent_dirs"

    echo "$all_content"
}

extract_text_rss() {
    # Split RSS/Atom items into one-line-per-item, strip tags, decode entities.
    sed 's/<item[> ]/\n<item>/gI; s/<entry[> ]/\n<entry>/gI' |
        sed -E 's/<[^>]+>//g' |
        sed 's/&lt;/</g; s/&gt;/>/g; s/&amp;/\&/g; s/&#39;/'"'"'/g; s/&quot;/"/g' |
        sed 's/[[:space:]]\{2,\}/ /g' |
        sed '/^[[:space:]]*$/d'
}

extract_text_html() {
    # Remove multi-line script/style blocks via address-range deletion.
    # Then strip remaining tags, decode entities, preserve block-level line breaks.
    sed '/<script/,/<\/script>/d' |
        sed '/<style/,/<\/style>/d' |
        sed 's/<\/\(p\|div\|li\|h[1-6]\|tr\|dt\|dd\|article\|section\)>/\n/gI' |
        sed 's/<br[^>]*>/\n/gI' |
        sed -E 's/<[^>]+>//g' |
        sed 's/&lt;/</g; s/&gt;/>/g; s/&amp;/\&/g; s/&#39;/'"'"'/g; s/&quot;/"/g; s/&nbsp;/ /g' |
        sed 's/[[:space:]]\{2,\}/ /g' |
        sed '/^[[:space:]]*$/d'
}

# Markdown: strip frontmatter, links, images. Keep text.
extract_text_md() {
    sed '/^---$/,/^---$/d' |
        sed -E 's/\[([^]]*)\]\([^)]*\)/\1/g' |
        sed -E 's/!\[([^]]*)\]\([^)]*\)/\1/g' |
        sed -E 's/^#+\s*//' |
        sed 's/[[:space:]]\{2,\}/ /g' |
        sed '/^[[:space:]]*$/d'
}

# MediaWiki API: extract wikitext from JSON, strip wiki markup.
extract_text_mwapi() {
    jq -r '.parse.wikitext["*"] // empty' 2>/dev/null |
        sed -E "s/\[\[([^]|]*\|)?([^]]*)\]\]/\2/g" |
        sed -E 's/\[https?:\/\/[^ ]* ([^]]*)\]/\1/g' |
        sed -E 's/\[https?:\/\/[^]]*\]//g' |
        sed "s/'''//g; s/''//g" |
        sed -E 's/^\*+\s*//' |
        sed -E 's/^=+\s*//; s/\s*=+$//' |
        sed 's/[[:space:]]\{2,\}/ /g' |
        sed '/^[[:space:]]*$/d'
}

HAS_NEW_MATCHES=false

scan_provider() {
    local provider="$1"
    local feed_type="$2"
    local url="$3"

    local content
    if [[ "$feed_type" == "ghdir" ]]; then
        content="$(fetch_scaleway_ghdir "$url")"
    else
        content="$(fetch_content "$url")"
    fi

    if [[ -z "$content" ]]; then
        FETCH_FAILURES=$((FETCH_FAILURES + 1))
        FETCH_FAILURE_LIST+="$provider "
        echo "  warning: failed to fetch $provider ($url)" >&2
        return
    fi

    local text
    case "$feed_type" in
        rss)   text="$(echo "$content" | extract_text_rss)" ;;
        html)  text="$(echo "$content" | extract_text_html)" ;;
        md)    text="$(echo "$content" | extract_text_md)" ;;
        ghdir) text="$(echo "$content" | extract_text_md)" ;;
        mwapi) text="$(echo "$content" | extract_text_mwapi)" ;;
    esac

    # Check for suspiciously low content (JS-rendered SPA or broken fetch)
    local line_count
    line_count="$(echo "$text" | wc -l | tr -d ' ')"
    if [[ "$line_count" -lt 5 ]]; then
        FETCH_FAILURES=$((FETCH_FAILURES + 1))
        FETCH_FAILURE_LIST+="$provider "
        echo "  warning: $provider returned only $line_count lines of text (possible JS-rendered SPA)" >&2
        return
    fi

    # Stage 1: find lines matching breaking keywords (case-insensitive, min 20 chars)
    local keyword_matches
    keyword_matches="$(echo "$text" | grep -iE "$KEYWORDS" | awk 'length >= 20' || true)"

    if [[ -z "$keyword_matches" ]]; then
        return
    fi

    # Stage 2: filter to lines also matching this provider's service keywords
    local service_kw="${SERVICE_KEYWORDS[$provider]:-}"
    local matches
    if [[ -n "$service_kw" ]]; then
        matches="$(echo "$keyword_matches" | grep -iE "$service_kw" || true)"
    else
        matches="$keyword_matches"
    fi

    if [[ -z "$matches" ]]; then
        return
    fi

    # Stage 3: drop lines matching this provider's exclusion keywords, UNLESS
    # the line also contains a strong API-path token (then keep it, defensive).
    local exclude_kw="${EXCLUDE_KEYWORDS[$provider]:-}"
    local strong_kw="${STRONG_API_TOKENS[$provider]:-}"
    if [[ -n "$exclude_kw" ]]; then
        local filtered=""
        while IFS= read -r line; do
            [[ -z "$line" ]] && continue
            if echo "$line" | grep -iE "$exclude_kw" >/dev/null; then
                # Line matches exclusion. Override if strong token present.
                if [[ -n "$strong_kw" ]] && echo "$line" | grep -iE "$strong_kw" >/dev/null; then
                    filtered+="$line"$'\n'
                else
                    # Log to stderr for calibration traceability
                    local short
                    short="$(echo "$line" | sed 's/^[[:space:]]*//' | cut -c1-120)"
                    echo "  [excluded: $provider] $short" >&2
                fi
            else
                filtered+="$line"$'\n'
            fi
        done <<< "$matches"
        matches="$(echo -n "$filtered" | sed '/^$/d')"
    fi

    if [[ -z "$matches" ]]; then
        return
    fi

    # Build set of seen hashes for this provider
    local -A seen_set=()
    while IFS= read -r h; do
        [[ -n "$h" ]] && seen_set["$h"]=1
    done < <(get_seen "$provider")

    local new_found=false
    local new_hashes=()
    local new_count=0

    while IFS= read -r line; do
        [[ -z "$line" ]] && continue
        local trimmed
        trimmed="$(echo "$line" | sed 's/^[[:space:]]*//;s/[[:space:]]*$//')"
        [[ -z "$trimmed" ]] && continue

        # Truncate very long lines to keep output readable
        if [[ ${#trimmed} -gt 200 ]]; then
            trimmed="${trimmed:0:200}..."
        fi

        local h
        h="$(hash_line "$provider:$trimmed")"

        if [[ -z "${seen_set[$h]:-}" ]]; then
            if [[ "$new_found" == false ]]; then
                echo "=== $provider ==="
                new_found=true
            fi
            # Cap output at 50 lines per provider but still track all hashes
            new_count=$((new_count + 1))
            if [[ $new_count -le 50 ]]; then
                echo "  $trimmed"
            elif [[ $new_count -eq 51 ]]; then
                echo "  ... (truncated, more matches found)"
            fi
            new_hashes+=("$h")
        fi
    done <<< "$matches"

    if [[ "$new_found" == true ]]; then
        echo ""
        HAS_NEW_MATCHES=true
        printf '%s\n' "${new_hashes[@]}" > "$WORK_DIR/changelog-new-$provider"
    fi
}

# --- main -------------------------------------------------------------------

echo "Checking provider changelogs for API-breaking keywords..."
echo "Keywords: ${KEYWORDS//|/, }"
echo ""

for entry in "${FEEDS[@]}"; do
    IFS='|' read -r provider feed_type url <<< "$entry"
    scan_provider "$provider" "$feed_type" "$url"
done

# --- fetch failure report ---------------------------------------------------

if [[ $FETCH_FAILURES -gt 0 ]]; then
    echo "=== FETCH FAILURES ($FETCH_FAILURES) ==="
    echo "  Providers: $FETCH_FAILURE_LIST"
    echo "  These providers are not being monitored this run."
    echo ""
fi

# --- update state -----------------------------------------------------------

if [[ "$UPDATE" == true ]]; then
    state="$(cat "$STATE_FILE")"
    for entry in "${FEEDS[@]}"; do
        IFS='|' read -r provider _ _ <<< "$entry"
        new_file="$WORK_DIR/changelog-new-$provider"
        if [[ -f "$new_file" ]]; then
            existing="$(echo "$state" | jq -c --arg p "$provider" '.[$p] // []')"
            while IFS= read -r h; do
                existing="$(echo "$existing" | jq -c --arg h "$h" '. + [$h] | unique')"
            done < "$new_file"
            state="$(echo "$state" | jq --arg p "$provider" --argjson v "$existing" '.[$p] = $v')"
        fi
    done
    # Atomic write: write to temp file then rename
    tmp_state="$(mktemp "$STATE_FILE.XXXXXX")"
    echo "$state" | jq '.' > "$tmp_state"
    mv "$tmp_state" "$STATE_FILE"
    echo "State file updated: $STATE_FILE"
fi

# --- exit code --------------------------------------------------------------

# Exit 1 only if new keyword matches found. Fetch failures are logged
# but do not trigger issue creation (those providers use OpenAPI validation).
if [[ "$HAS_NEW_MATCHES" == true ]]; then
    exit 1
fi

exit 0
