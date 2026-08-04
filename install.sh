#!/usr/bin/env sh
set -eu

BIN_NAMES="agent-session-find agent-skill-validate"
PROFILE=${PROFILE:-release}
BUILD=1

if [ -n "${BIN_DIR:-}" ]; then
  INSTALL_DIR=$BIN_DIR
elif [ -n "${PREFIX:-}" ]; then
  INSTALL_DIR=$PREFIX/bin
else
  : "${HOME:?HOME must be set unless BIN_DIR or PREFIX is provided}"
  INSTALL_DIR=$HOME/.local/bin
fi

usage() {
  cat <<'EOF'
usage: ./install.sh [--bin-dir DIR] [--prefix DIR] [--profile release|debug] [--no-build]

Build and install the Rust agent-session-find and agent-skill-validate binaries.

Options:
  --bin-dir DIR          Install directly into DIR.
  --prefix DIR           Install into DIR/bin.
  --profile PROFILE      Build/copy target/release or target/debug. Default: release.
  --no-build             Copy an existing target binary without running cargo build.
  -h, --help             Show this help.
EOF
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --bin-dir)
      shift
      [ "$#" -gt 0 ] || { echo "install.sh: --bin-dir needs a value" >&2; exit 2; }
      INSTALL_DIR=$1
      ;;
    --prefix)
      shift
      [ "$#" -gt 0 ] || { echo "install.sh: --prefix needs a value" >&2; exit 2; }
      INSTALL_DIR=$1/bin
      ;;
    --profile)
      shift
      [ "$#" -gt 0 ] || { echo "install.sh: --profile needs a value" >&2; exit 2; }
      PROFILE=$1
      ;;
    --no-build)
      BUILD=0
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "install.sh: unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
  shift
done

case "$PROFILE" in
  release|debug) ;;
  *)
    echo "install.sh: --profile must be release or debug" >&2
    exit 2
    ;;
esac

SCRIPT_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)

if [ "$BUILD" -eq 1 ]; then
  if [ "$PROFILE" = release ]; then
    cargo build --release --manifest-path "$SCRIPT_DIR/Cargo.toml" --bins
  else
    cargo build --manifest-path "$SCRIPT_DIR/Cargo.toml" --bins
  fi
fi

mkdir -p "$INSTALL_DIR"

for BIN_NAME in $BIN_NAMES; do
  if [ -n "${CARGO_TARGET_DIR:-}" ]; then
    SOURCE_BIN=$CARGO_TARGET_DIR/$PROFILE/$BIN_NAME
  else
    SOURCE_BIN=$SCRIPT_DIR/target/$PROFILE/$BIN_NAME
  fi
  if [ ! -x "$SOURCE_BIN" ]; then
    echo "install.sh: missing built binary: $SOURCE_BIN" >&2
    echo "install.sh: run without --no-build, or build it first." >&2
    exit 1
  fi

  TMP_BIN=$INSTALL_DIR/.$BIN_NAME.tmp.$$
  trap 'rm -f "$TMP_BIN"' EXIT HUP INT TERM
  cp "$SOURCE_BIN" "$TMP_BIN"
  chmod 755 "$TMP_BIN"
  mv "$TMP_BIN" "$INSTALL_DIR/$BIN_NAME"
  trap - EXIT HUP INT TERM

  echo "Installed $BIN_NAME to $INSTALL_DIR/$BIN_NAME"
done

case ":${PATH:-}:" in
  *":$INSTALL_DIR:"*) ;;
  *)
    echo "Add this to PATH if needed:"
    echo "  export PATH=\"$INSTALL_DIR:\$PATH\""
    ;;
esac
