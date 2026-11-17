#!/usr/bin/env bash
# Compatibility entry point; the installer now uses community-scripts menus.
if [[ "$#" -gt 0 ]]; then
  echo 'Use bash ct/dakia.sh and var_dakia_origin=https://mail.example.com; the old CLI flags are no longer supported.' >&2
  exit 1
fi
exec bash "$(dirname "${BASH_SOURCE[0]}")/../ct/dakia.sh"
