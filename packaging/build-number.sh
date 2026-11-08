# shellcheck shell=bash
# Sourced by build-app.sh, appstore/build.sh and release.sh: the CFBundleVersion build number.
#
# Sparkle and App Store Connect both need every release's CFBundleVersion to be higher than
# the last one. The default is the commit count of HEAD, which only rises along main. It is
# wrong on a shallow clone (CI's default depth of 1 gives "1") and can go backwards on a
# branch that isn't part of main, so both cases stop the build.
#
#   VW_BUILD_NUMBER=<n>            use this number; no git checks (local tests, e.g. docs/sparkle-e2e.md)
#   VW_BUILD_FROM_ANY_COMMIT=1     count commits even when HEAD isn't on origin/main: a deliberate
#                                  hotfix branch, or a local test build. Make sure the number is
#                                  still higher than the last release's.
#
# Sets BUILD_NUMBER, or prints why not and returns 1.
vw_build_number() {
  local root="$1"
  if [[ -n "${VW_BUILD_NUMBER:-}" ]]; then
    if ! [[ "$VW_BUILD_NUMBER" =~ ^[0-9]+$ ]]; then
      echo "error: VW_BUILD_NUMBER must be a whole number, not '$VW_BUILD_NUMBER'" >&2
      return 1
    fi
    BUILD_NUMBER="$VW_BUILD_NUMBER"
    return 0
  fi
  if ! git -C "$root" rev-parse --git-dir >/dev/null 2>&1; then
    echo "error: not a git checkout, so there is no commit count for CFBundleVersion; set VW_BUILD_NUMBER" >&2
    return 1
  fi
  if [[ "$(git -C "$root" rev-parse --is-shallow-repository)" == true ]]; then
    echo "error: this is a shallow clone, so the commit count (CFBundleVersion) would be too low." >&2
    echo "       Fetch the full history (git fetch --unshallow; in Actions: fetch-depth: 0)" >&2
    echo "       or set VW_BUILD_NUMBER." >&2
    return 1
  fi
  if [[ -z "${VW_BUILD_FROM_ANY_COMMIT:-}" ]]; then
    if ! git -C "$root" rev-parse -q --verify refs/remotes/origin/main >/dev/null; then
      echo "error: no origin/main to check HEAD against (git fetch origin main), or set" >&2
      echo "       VW_BUILD_FROM_ANY_COMMIT=1 / VW_BUILD_NUMBER (see packaging/build-number.sh)" >&2
      return 1
    fi
    if ! git -C "$root" merge-base --is-ancestor HEAD refs/remotes/origin/main; then
      echo "error: HEAD ($(git -C "$root" rev-parse --short HEAD)) is not on origin/main, so its commit" >&2
      echo "       count may not be higher than the last release's. Release from main (merge, then" >&2
      echo "       git fetch). For a deliberate hotfix or a local test build set" >&2
      echo "       VW_BUILD_FROM_ANY_COMMIT=1, or VW_BUILD_NUMBER=<n>." >&2
      return 1
    fi
  fi
  # shellcheck disable=SC2034 # read by the scripts that source this file
  BUILD_NUMBER="$(git -C "$root" rev-list --count HEAD)"
}
