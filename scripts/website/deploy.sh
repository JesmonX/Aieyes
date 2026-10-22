#!/bin/sh
# Build, verify and publish only the static website to the authorized proxyus host.
set -eu
PROJECT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$PROJECT_DIR"
node scripts/website/build.mjs
node scripts/website/test.cjs
RELEASE_NAME=$(date -u +%Y%m%dT%H%M%SZ)
SSH_OPTIONS='-o ClearAllForwardings=yes -o BatchMode=yes -o StrictHostKeyChecking=yes -o UpdateHostKeys=no -o ConnectTimeout=15'
ssh $SSH_OPTIONS proxyus bash -s -- "$RELEASE_NAME" <<'REMOTE'
set -euo pipefail
release=$1
if ! command -v caddy >/dev/null; then
    sudo -n apt-get update -qq
    sudo -n env DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends caddy
fi
sudo -n install -d -m 0755 /var/www/aieyes/releases /etc/caddy/sites
sudo -n install -d -m 0755 -o "$(id -un)" -g "$(id -gn)" "/var/www/aieyes/releases/$release"
REMOTE
rsync -rtz -e "ssh $SSH_OPTIONS" dist/website/ "proxyus:/var/www/aieyes/releases/$RELEASE_NAME/"
rsync -t -e "ssh $SSH_OPTIONS" scripts/website/Caddyfile "proxyus:/var/www/aieyes/releases/$RELEASE_NAME/.caddy-source"
rsync -t -e "ssh $SSH_OPTIONS" scripts/website/CaddyGlobal "proxyus:/var/www/aieyes/releases/$RELEASE_NAME/.caddy-global-source"
ssh $SSH_OPTIONS proxyus bash -s -- "$RELEASE_NAME" <<'REMOTE'
set -euo pipefail
release=$1
base=/var/www/aieyes
caddy_user=$(systemctl show -p User --value caddy)
caddy_group=$(systemctl show -p Group --value caddy)
test -n "$caddy_user" || caddy_user=caddy
test -n "$caddy_group" || caddy_group=caddy
sudo -n install -d -m 0750 -o "$caddy_user" -g "$caddy_group" /var/log/caddy
chmod -R u=rwX,go=rX "$base/releases/$release"
backup="/etc/caddy/aieyes-backups/$release"
sudo -n install -d -m 0700 "$backup"
sudo -n cp /etc/caddy/Caddyfile "$backup/Caddyfile"
if test -f /etc/caddy/sites/aieyes.caddy; then sudo -n cp /etc/caddy/sites/aieyes.caddy "$backup/aieyes.caddy"; fi
if test -f /etc/caddy/aieyes-global-options; then sudo -n cp /etc/caddy/aieyes-global-options "$backup/global-options"; fi
previous=$(readlink "$base/current" || true)
rollback() {
    echo 'Health check failed; restoring the preceding website and Caddy configuration.' >&2
    if test -n "$previous"; then
        sudo -n ln -s "$previous" "$base/.rollback-$release"
        sudo -n mv -Tf "$base/.rollback-$release" "$base/current"
    else
        sudo -n rm -f "$base/current"
    fi
    sudo -n cp "$backup/Caddyfile" /etc/caddy/Caddyfile
    if sudo -n test -f "$backup/aieyes.caddy"; then sudo -n cp "$backup/aieyes.caddy" /etc/caddy/sites/aieyes.caddy; else sudo -n rm -f /etc/caddy/sites/aieyes.caddy; fi
    if sudo -n test -f "$backup/global-options"; then sudo -n cp "$backup/global-options" /etc/caddy/aieyes-global-options; else sudo -n rm -f /etc/caddy/aieyes-global-options; fi
    sudo -n caddy reload --config /etc/caddy/Caddyfile --force || true
}
trap rollback ERR
sudo -n install -m 0644 "$base/releases/$release/.caddy-source" /etc/caddy/sites/aieyes.caddy
sudo -n install -m 0644 "$base/releases/$release/.caddy-global-source" /etc/caddy/aieyes-global-options
# The source config is outside the served directory; only static build output is public.
rm "$base/releases/$release/.caddy-source"
rm "$base/releases/$release/.caddy-global-source"
sudo -n python3 - <<'PY'
from pathlib import Path
path = Path('/etc/caddy/Caddyfile')
text = path.read_text()
directive = 'import /etc/caddy/aieyes-global-options'
if directive not in text:
    lines = text.splitlines(keepends=True)
    first = next((i for i, line in enumerate(lines) if line.strip() and not line.lstrip().startswith('#')), None)
    if first is not None and lines[first].strip() == '{':
        lines.insert(first + 1, '\t' + directive + '\n')
        text = ''.join(lines)
    elif first is not None and lines[first].lstrip().startswith('{'):
        raise SystemExit('An inline global block needs an explicit merge before deployment.')
    else:
        text = '{\n\t' + directive + '\n}\n\n' + text
    path.write_text(text)
PY
if ! sudo -n grep -Fq 'import /etc/caddy/sites/*.caddy' /etc/caddy/Caddyfile; then
    printf '\nimport /etc/caddy/sites/*.caddy\n' | sudo -n tee -a /etc/caddy/Caddyfile >/dev/null
fi
sudo -n caddy validate --config /etc/caddy/Caddyfile
sudo -n ln -s "releases/$release" "$base/.current-$release"
sudo -n mv -Tf "$base/.current-$release" "$base/current"
sudo -n systemctl enable --now caddy
sudo -n caddy reload --config /etc/caddy/Caddyfile --force
curl --fail --silent --show-error --retry 8 --retry-all-errors --retry-delay 3 --max-time 15 https://aieyes.2h3ngy4n.space/ -o /dev/null
curl --fail --silent --show-error --max-time 15 https://aieyes.2h3ngy4n.space/zh-CN/ -o /dev/null
trap - ERR
# Failed uploads never displace the preceding successful rollback versions.
sudo -n install -d -m 0755 "$base/completed"
sudo -n touch "$base/completed/$release"
find "$base/completed" -mindepth 1 -maxdepth 1 -type f -printf '%f\n' | sort -r | tail -n +4 | while IFS= read -r old; do
    case "$old" in [0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9]T[0-9][0-9][0-9][0-9][0-9][0-9]Z) sudo -n rm -rf -- "$base/releases/$old"; sudo -n rm -f -- "$base/completed/$old" ;; esac
done
printf 'Published release: %s\n' "$release"
readlink "$base/current"
REMOTE
curl --fail --silent --show-error --max-time 20 https://aieyes.2h3ngy4n.space/ -o /dev/null
curl --fail --silent --show-error --max-time 20 https://aieyes.2h3ngy4n.space/zh-CN/ -o /dev/null
printf 'Verified public website: https://aieyes.2h3ngy4n.space\n'
