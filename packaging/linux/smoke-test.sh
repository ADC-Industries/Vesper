#!/usr/bin/env bash
# Smoke test of the .debs in throwaway Ubuntu containers (podman, rootless), never on the host:
#   1. apt installs the server, the Launcher, the bundled IDN Node and the tray with their
#      dependencies; `vesper --help` runs; the vesper user, its groups and the data folder
#      exist; systemd accepts the units; nothing in `ldd` is missing (the IDN programs and the
#      tray too); the tray starts at log-in (XDG autostart).
#   2. The container boots systemd: the service starts by itself (no GPU in the container:
#      Mesa's software renderer), answers on :5555 and serves the control UI; the Launcher
#      runs and knows the installed version (dpkg's); a display box (vesper-display enabled)
#      keeps its unit usable across apt remove + install, and the Launcher runs again; then
#      purge keeps /var/lib/vesper.
#      The bundled IDN Node's "updates managed by Vesper" marker is there, the Launcher puts it
#      back when it is deleted, and purging the package removes it.
#      The bundled IDN Node is one of the Launcher's products (its package, vesper-idn-node, a
#      package of programs): a newer one, signed with a throwaway key, is uploaded to the
#      Launcher's local API and installed by its update action, as from the site's channel.
#      The Launcher's diagnostics run the bundled `idn doctor`. Without the server (purged),
#      the Launcher alone keeps running: a display box or a control PC (the Launcher-only
#      install).
#      With a previous package, it is installed first and upgraded from (an older package with
#      the Launcher inside: the upgrade moves it to its own package; or one without the
#      Launcher: the upgrade starts it). First through the Launcher, as the site's channel
#      would: the Launcher (and the server's product file, which revision 1 doesn't have) is
#      added by hand, and the server's package, signed with a throwaway key
#      (packaging/linux/smoke-sign.py, python3-cryptography), is uploaded to its local API and
#      installed by its update action; then apt installs every package, as by hand.
#
#   packaging/linux/smoke-test.sh [packaging/out/vesper_<version>_amd64.deb [previous.deb]]
#
# The Launcher's, the IDN Node's and the tray's packages are the newest in the server package's
# folder.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
deb="${1:-$(ls -t "$root"/packaging/out/vesper_*_amd64.deb | head -n1)}"
deb="$(realpath "$deb")"
dir="$(dirname "$deb")"
newest() { ls -t "$dir"/"$1"_*_amd64.deb 2>/dev/null | head -n1; }
launcher_deb="$(newest vesper-launcher)"
idn_deb="$(newest vesper-idn-node)"
tray_deb="$(newest vesper-launcher-tray)"
for f in "$launcher_deb" "$idn_deb" "$tray_deb"; do
  [[ -f "$f" ]] || { echo "missing the Launcher's, IDN Node's or tray's package next to $deb" >&2; exit 1; }
done
debs=(-v "$deb:/tmp/vesper.deb:ro" -v "$launcher_deb:/tmp/vesper-launcher.deb:ro" -v "$idn_deb:/tmp/vesper-idn-node.deb:ro" -v "$tray_deb:/tmp/vesper-launcher-tray.deb:ro")
# A newer bundled IDN Node package (the same programs, its revision raised to 99), signed with a
# throwaway key (packaging/linux/smoke-sign.py, python3-cryptography): the Launcher updates it.
idn_sign_dir="$(mktemp -d)"
trap 'rm -rf -- "$idn_sign_dir"' EXIT
idn_new="$(dpkg-deb -f "$idn_deb" Version | sed 's/-[^-]*$//')-99"
dpkg-deb -R "$idn_deb" "$idn_sign_dir/pkg"
sed -i "s/^Version: .*/Version: $idn_new/" "$idn_sign_dir/pkg/DEBIAN/control"
dpkg-deb --root-owner-group -Zxz --build "$idn_sign_dir/pkg" "$idn_sign_dir/idn.deb" >/dev/null
rm -rf -- "$idn_sign_dir/pkg"
python3 -I "$root/packaging/linux/smoke-sign.py" "$idn_sign_dir" "$idn_sign_dir/idn.deb" vesper-idn-node \
  "$idn_new" "linux-$(uname -m)"
debs+=(-v "$idn_sign_dir:/tmp/idnsign:ro")
previous="${2:-}"
prev_mount=()
if [[ -n "$previous" ]]; then
  previous="$(realpath "$previous")"
  sign_dir="$(mktemp -d)"
  trap 'rm -rf -- "$sign_dir" "$idn_sign_dir"' EXIT
  python3 -I "$root/packaging/linux/smoke-sign.py" "$sign_dir" "$deb" vesper \
    "$(dpkg-deb -f "$deb" Version)" "linux-$(uname -m)"
  mv "$sign_dir/$(basename "$deb").minisig" "$sign_dir/vesper.minisig"
  prev_mount=(-v "$previous:/tmp/previous.deb:ro" -v "$sign_dir:/tmp/sign:ro")
fi
release="$(. /etc/os-release && echo "$VERSION_ID")"
image="docker.io/library/ubuntu:$release"
name="vesper-smoke-$$"

echo "== $deb (+ $(basename "$launcher_deb"), $(basename "$idn_deb"), $(basename "$tray_deb")) on $image"
podman run --rm "${debs[@]}" "$image" bash -euc '
  export DEBIAN_FRONTEND=noninteractive
  apt-get update -qq
  apt-get install -y -qq --no-install-recommends /tmp/vesper.deb /tmp/vesper-launcher.deb /tmp/vesper-idn-node.deb /tmp/vesper-launcher-tray.deb curl >/dev/null
  echo "-- vesper --help"; vesper --help | head -n 5
  echo "-- vesper-launcher join --help"; vesper-launcher join --help | head -n 3
  echo "-- idn-node"; /usr/lib/vesper-idn-node/idn-node version
  echo "-- IDN updates managed by: $(head -n1 /usr/lib/vesper-idn-node/idn-updates-managed-by)"
  test "$(head -n1 /usr/lib/vesper-idn-node/idn-updates-managed-by)" = Vesper
  echo "-- launcher unit"; systemd-analyze verify /usr/lib/systemd/system/vesper-launcher.service && echo "launcher unit OK"
  echo "-- products"; cat /etc/vesper/launcher.json /etc/vesper/launcher.d/vesper.json /etc/vesper/launcher.d/vesper-idn-node.json | grep -o "\"product\": \"[^\"]*\""
  echo "-- tray autostart"; grep -q "^Exec=vesper-launcher-tray" /etc/xdg/autostart/vesper-launcher-tray.desktop && echo "tray starts at log-in"
  for b in /usr/bin/vesper-launcher /usr/bin/vesper-launcher-tray /usr/lib/vesper-idn-node/*; do
    ! ldd "$b" | grep "not found" || { echo "$b: missing libraries"; exit 1; }
  done
  echo "-- licences"; ls /usr/share/doc/vesper-idn-node/
  echo "-- user";  id vesper
  echo "-- data";  stat -c "%U:%G %a %n" /var/lib/vesper
  echo "-- unit";  systemd-analyze verify /usr/lib/systemd/system/vesper.service && echo "unit OK"
  echo "-- display unit"; systemd-analyze verify /usr/lib/systemd/system/vesper-display.service && echo "display unit OK"
  echo "-- enabled"; ls /etc/systemd/system/multi-user.target.wants/ | grep vesper
  echo "-- libraries"; ldd /usr/bin/vesper | grep -c "=> /" ; ! ldd /usr/bin/vesper | grep "not found"
'

echo "== boot test (systemd in the container)"
cleanup() {
  podman rm -f "$name" >/dev/null 2>&1 || true
  [[ -z "${sign_dir:-}" ]] || rm -rf -- "$sign_dir"
  rm -rf -- "$idn_sign_dir"
}
trap cleanup EXIT
# --privileged stays inside rootless podman's user namespace; systemd needs it to set up the
# unit's sandbox (namespaces, capabilities).
# tmp.mount masked: Ubuntu's systemd (259 on 26.04) mounts a fresh tmpfs on /tmp at boot,
# hiding the packages mounted there.
podman run -d --name "$name" --privileged --systemd=always "${debs[@]}" "${prev_mount[@]}" "$image" \
  bash -c 'apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq systemd systemd-sysv >/dev/null && ln -sf /dev/null /etc/systemd/system/tmp.mount && exec /sbin/init' >/dev/null
for _ in $(seq 120); do
  podman exec "$name" test -d /run/systemd/system 2>/dev/null && break
  sleep 2
done
podman exec -e IDN_NEW="$idn_new" "$name" bash -euc '
  export DEBIAN_FRONTEND=noninteractive
  # The image forbids starting services from maintainer scripts; a real machine does not.
  rm -f /usr/sbin/policy-rc.d
  launcher_up() {
    for i in $(seq 30); do curl -fsS -o /dev/null http://127.0.0.1:5556/launcher/v1/status 2>/dev/null && return 0; sleep 1; done
    return 1
  }
  if [ -f /tmp/previous.deb ]; then
    apt-get install -y -qq --no-install-recommends /tmp/previous.deb curl >/dev/null
    echo "-- previous: $(dpkg-query -W -f="\${Version}" vesper), service $(systemctl is-active vesper.service || true)"
    # Through the Launcher, like an update from the site channel: from its local API.
    new="$(dpkg-deb -f /tmp/vesper.deb Version)"
    apt-get install -y -qq --no-install-recommends /tmp/vesper-launcher.deb /tmp/vesper-idn-node.deb >/dev/null
    install -D -m 0644 /tmp/sign/smoke.pub /etc/vesper/update-keys/smoke.pub
    # (The server as a product of the Launcher: the file the new package brings.)
    if [ ! -f /etc/vesper/launcher.d/vesper.json ]; then
      dpkg-deb --fsys-tarfile /tmp/vesper.deb | tar -xO ./etc/vesper/launcher.d/vesper.json > /etc/vesper/launcher.d/vesper.json
    fi
    systemctl restart vesper-launcher.service
    launcher_up
    curl -fsS -X POST -H "X-Vesper-Signature: $(base64 -w0 /tmp/sign/vesper.minisig)" \
      --data-binary @/tmp/vesper.deb "http://127.0.0.1:5556/launcher/v1/packages?file=vesper_${new}_amd64.deb" >/dev/null
    outcome="$(curl -sS --max-time 300 -X POST -H "Content-Type: application/json" \
      -d "{\"type\":\"update\",\"product\":\"vesper\",\"version\":\"$new\"}" http://127.0.0.1:5556/launcher/v1/actions)"
    echo "-- updated by the Launcher: $(echo "$outcome" | grep -o "\"result\":\"[a-zA-Z]*\""), vesper $(dpkg-query -W -f="\${Version} \${db:Status-Status}" vesper), service $(systemctl is-active vesper.service || true)"
    echo "$outcome" | grep -q "\"result\":\"installed\"" || { echo "$outcome"; exit 1; }
    test "$(dpkg-query -W -f="\${Version} \${db:Status-Status}" vesper)" = "$new installed"
    apt-get install -y -qq --no-install-recommends /tmp/vesper.deb /tmp/vesper-launcher.deb /tmp/vesper-idn-node.deb >/dev/null
    echo "-- upgraded to $(dpkg-query -W -f="\${Version}" vesper), launcher $(dpkg-query -W -f="\${Version}" vesper-launcher)"
  fi
  apt-get install -y -qq --no-install-recommends /tmp/vesper.deb /tmp/vesper-launcher.deb /tmp/vesper-idn-node.deb curl >/dev/null
  launcher_up
  echo "-- launcher: $(systemctl is-active vesper-launcher.service)"; systemctl is-active --quiet vesper-launcher.service
  # (Adopted from dpkg at its next health check once dpkg has finished configuring.)
  installed="$(dpkg-query -W -f="\${Version}" vesper)"
  for i in $(seq 30); do
    status="$(curl -fsS http://127.0.0.1:5556/launcher/v1/status)"
    echo "$status" | grep -q "\"version\":\"$installed\"" && break
    sleep 1
  done
  echo "-- launcher knows the installed version: $(echo "$status" | grep -o "\"version\":\"[^\"]*\"")"
  echo "$status" | grep -q "\"version\":\"$installed\""
  echo "-- the Launcher knows itself: $(echo "$status" | grep -o "\"product\":\"vesper-launcher\"[^}]*\"version\":\"[^\"]*\"" | grep -o "\"version\":\"[^\"]*\"")"
  echo "$status" | grep -q "\"product\":\"vesper-launcher\""
  # The bundled IDN Node is one of the Launcher'"'"'s products (H04-2 / R3-2): it knows dpkg'"'"'s
  # version, and installs a newer signed package from its local API, as from the site'"'"'s channel.
  idn_have="$(dpkg-query -W -f="\${Version}" vesper-idn-node)"
  for i in $(seq 30); do
    status="$(curl -fsS http://127.0.0.1:5556/launcher/v1/status)"
    echo "$status" | grep -o "\"product\":\"vesper-idn-node\"[^}]*" | grep -qF "\"version\":\"$idn_have\"" && break
    sleep 1
  done
  echo "-- the Launcher knows the IDN Node: $(echo "$status" | grep -o "\"product\":\"vesper-idn-node\"[^}]*" | grep -o "\"version\":\"[^\"]*\"")"
  echo "$status" | grep -o "\"product\":\"vesper-idn-node\"[^}]*" | grep -qF "\"version\":\"$idn_have\""
  install -D -m 0644 /tmp/idnsign/smoke.pub /etc/vesper/update-keys/smoke-idn.pub
  systemctl restart vesper-launcher.service
  launcher_up
  curl -fsS -X POST -H "X-Vesper-Signature: $(base64 -w0 /tmp/idnsign/idn.deb.minisig)" \
    --data-binary @/tmp/idnsign/idn.deb "http://127.0.0.1:5556/launcher/v1/packages?file=vesper-idn-node_${IDN_NEW}_amd64.deb" >/dev/null
  outcome="$(curl -sS --max-time 300 -X POST -H "Content-Type: application/json" \
    -d "{\"type\":\"update\",\"product\":\"vesper-idn-node\",\"version\":\"$IDN_NEW\"}" http://127.0.0.1:5556/launcher/v1/actions)"
  echo "-- IDN Node updated by the Launcher: $(echo "$outcome" | grep -o "\"result\":\"[a-zA-Z]*\""), vesper-idn-node $(dpkg-query -W -f="\${Version} \${db:Status-Status}" vesper-idn-node)"
  echo "$outcome" | grep -q "\"result\":\"installed\"" || { echo "$outcome"; exit 1; }
  test "$(dpkg-query -W -f="\${Version} \${db:Status-Status}" vesper-idn-node)" = "$IDN_NEW installed"
  # The "updates managed by Vesper" marker beside the IDN Node: the Launcher puts it back.
  rm /usr/lib/vesper-idn-node/idn-updates-managed-by
  for i in $(seq 30); do test -f /usr/lib/vesper-idn-node/idn-updates-managed-by && break; sleep 1; done
  echo "-- marker put back by the Launcher: $(head -n1 /usr/lib/vesper-idn-node/idn-updates-managed-by)"
  test "$(head -n1 /usr/lib/vesper-idn-node/idn-updates-managed-by)" = Vesper
  diag="$(curl -fsS --max-time 90 http://127.0.0.1:5556/launcher/v1/diagnostics)"
  echo "-- diagnostics: idn doctor ran: $(echo "$diag" | grep -o "\"ran\":[a-z]*"); $(echo "$diag" | grep -o "\"headline\":\"[^\"]*\"")"
  echo "$diag" | grep -q "\"ran\":true"
  for i in $(seq 60); do curl -fsS -o /dev/null http://127.0.0.1:5555/ 2>/dev/null && break; sleep 1; done
  echo "-- service: $(systemctl is-active vesper.service)"; systemctl is-active --quiet vesper.service
  echo "-- UI";  curl -fsS http://127.0.0.1:5555/ | grep -o "<title>[^<]*</title>"
  echo "-- API"; curl -fsS http://127.0.0.1:5555/api/v1/server; echo
  echo "-- log"; journalctl -u vesper --no-pager -o cat | grep -E "Started|renderer ready|listening|pairing code|ERROR|Error" | cut -c1-160
  echo "-- data"; ls /var/lib/vesper
  systemd-analyze security vesper.service --no-pager | tail -n 1
  # A display box (vesper-display enabled instead) survives apt remove + install: postrm
  # masks both units on remove, and postinst must unmask both again.
  systemctl disable --quiet --now vesper.service
  systemctl enable --quiet vesper-display.service
  apt-get remove -y -qq vesper >/dev/null
  apt-get install -y -qq --no-install-recommends /tmp/vesper.deb /tmp/vesper-launcher.deb >/dev/null
  state="$(systemctl is-enabled vesper-display.service 2>/dev/null || true)"
  echo "-- display unit after remove + install: $state"; test "$state" = enabled
  launcher_up
  echo "-- launcher after remove + install: $(systemctl is-active vesper-launcher.service)"
  systemctl is-active --quiet vesper-launcher.service
  systemctl disable --quiet --now vesper-display.service
  systemctl enable --quiet vesper.service
  apt-get purge -y -qq vesper >/dev/null
  echo "-- after purge"; test -d /var/lib/vesper && echo "/var/lib/vesper kept"
  # The Launcher alone (a display box, a control PC): still running, managing only itself.
  systemctl restart vesper-launcher.service
  launcher_up
  status="$(curl -fsS http://127.0.0.1:5556/launcher/v1/status)"
  echo "-- Launcher-only: $(systemctl is-active vesper-launcher.service), products: $(echo "$status" | grep -o "\"product\":\"[^\"]*\"" | tr "\n" " ")"
  ! echo "$status" | grep -q "\"product\":\"vesper\""
  apt-get purge -y -qq vesper-launcher vesper-idn-node >/dev/null
  test ! -d /var/lib/vesper-launcher && echo "-- Launcher purged: its state is gone"
  test ! -e /usr/lib/vesper-idn-node/idn-updates-managed-by && echo "-- IDN Node purged: its marker is gone"
'
echo "== smoke test passed"
