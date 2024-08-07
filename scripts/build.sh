#!/usr/bin/env bash
source scripts/_common.sh

# allow for overriding arguments
export FM_FED_SIZE=${1:-4}

# If $TMP contains '/nix-shell.' it is already unique to the
# nix shell instance, and appending more characters to it is
# pointless. It only gets us closer to the 108 character limit
# for named unix sockets (https://stackoverflow.com/a/34833072),
# so let's not do it.

if [[ "${TMP:-}" == *"/nix-shell."* ]]; then
  FM_TEST_DIR="${2-$TMP}/fm-$(LC_ALL=C tr -dc A-Za-z0-9 </dev/urandom | head -c 4 || true)"
else
  FM_TEST_DIR="${2-"$(mktemp --tmpdir -d XXXXX)"}"
fi

export FM_TEST_DIR
export FM_LOGS_DIR="$FM_TEST_DIR/logs"

echo "Setting up env variables in $FM_TEST_DIR"

mkdir -p "$FM_TEST_DIR"

make_fm_test_marker

# Symlink $FM_TEST_DIR to local gitignored target/ directory so they're easier to find
rm -f target/devimint
mkdir -p target
ln -s $FM_TEST_DIR target/devimint

# Builds the rust executables and sets environment variables
SRC_DIR="$( cd -- "$( dirname -- "${BASH_SOURCE[0]}" )/.." &> /dev/null && pwd )"
cd $SRC_DIR || exit 1
# Note: Respect 'CARGO_PROFILE' that crane uses

build_workspace
add_target_dir_to_path
