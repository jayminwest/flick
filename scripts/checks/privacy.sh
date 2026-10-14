#!/usr/bin/env bash
# Gate check:privacy: the repo is public, so no tracked or new file may name a real
# tailnet, Tailscale address or personal machine. Use placeholders instead:
# `<host>.example.ts.net`, 100.64.0.x, fd7a:115c:a1e0::x, `my-macbook-pro`, `me@server`.
# Fails on:
#   - a tailnet MagicDNS suffix (`tail<hex>.ts.net`)
#   - an address in 100.64.0.0/10 outside 100.64.0.0/24 and Tailscale's documented
#     fixed ones (100.100.100.100, 100.101.102.103, 100.127.255.254/255)
#   - a full fd7a:115c:a1e0::/48 address (all eight groups written out)
#   - the owner's personal machine names, ssh user or home directory
set -euo pipefail
cd "$(dirname "$0")/../.."

# This script names the patterns itself, so it is not scanned.
git ls-files -z -co --exclude-standard | grep -zv '^scripts/checks/privacy\.sh$' |
  xargs -0 perl -ne '
    BEGIN { $n = 0; $files = @ARGV; %ok = map { $_ => 1 } qw(100.100.100.100 100.101.102.103 100.127.255.254 100.127.255.255) }
    next if -B $ARGV;
    my @hits;
    push @hits, $1 while /\b(tail[0-9a-f]{4,}\.ts\.net)\b/gi;
    while (/\b(100\.(?:6[4-9]|[7-9]\d|1[01]\d|12[0-7])\.\d{1,3}\.\d{1,3})\b/g) {
      push @hits, $1 unless $1 =~ /^100\.64\.0\./ || $ok{$1};
    }
    push @hits, $1 while /\b(fd7a:115c:a1e0(?::[0-9a-f]{1,4}){5})\b/gi;
    push @hits, $1 while /(jaymins?-(?:macbook|mac-pro|iphone|ipad)[\w-]*|mac-pro-jaymin|\bjaymin@|\/Users\/jaymin\w*)/gi;
    for (@hits) { print "error: $ARGV:$.: $_  [use a placeholder; see scripts/checks/privacy.sh]\n"; $n++ }
    close ARGV if eof;
    END { print "check:privacy: $files files clean\n" unless $n; exit($n ? 1 : 0) }
  '
