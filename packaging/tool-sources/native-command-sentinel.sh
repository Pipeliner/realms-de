#!/bin/sh
# Deny package-build VCS/network commands while classifying pinned metadata.
set -eu

sentinel_command=$(basename "$0")

is_exact_git_metadata() {
    case $# in
        2)
            [ "$1" = status ] && [ "$2" = --porcelain ] && return 0
            [ "$1" = rev-parse ] && [ "$2" = HEAD ] && return 0
            ;;
        3)
            [ "$1" = status ] && [ "$2" = --porcelain ] \
                && [ "$3" = --untracked-files=all ] && return 0
            [ "$1" = rev-parse ] && [ "$2" = --short ] \
                && [ "$3" = HEAD ] && return 0
            [ "$1" = log ] && [ "$2" = -1 ] \
                && { [ "$3" = '--pretty=format:%an' ] \
                    || [ "$3" = '--pretty=format:%ae' ]; } && return 0
            [ "$1" = describe ] && [ "$2" = --tags ] \
                && [ "$3" = HEAD ] && return 0
            [ "$1" = symbolic-ref ] && [ "$2" = --short ] \
                && [ "$3" = HEAD ] && return 0
            ;;
        4)
            [ "$1" = show ] && [ "$2" = '--pretty=format:%ct' ] \
                && [ "$3" = --date=raw ] && [ "$4" = -s ] && return 0
            [ "$1" = tag ] && [ "$2" = -l ] \
                && [ "$3" = --contains ] && [ "$4" = HEAD ] && return 0
            [ "$1" = describe ] && [ "$2" = --tags ] \
                && [ "$3" = --abbrev=0 ] && [ "$4" = HEAD ] && return 0
            ;;
    esac
    return 1
}

if [ "$sentinel_command" = git ] \
    && [ "$PWD" = "${REALM_EXPECTED_STARSHIP_SOURCE:-}" ] \
    && is_exact_git_metadata "$@"; then
    printf 'git-metadata-denied|cwd=%s|argc=%s|args=%s\n' \
        "$PWD" "$#" "$*" >>"${REALM_SENTINEL_LOG:?}"
    exit 97
fi

printf 'forbidden|command=%s|cwd=%s|argc=%s|args=%s\n' \
    "$sentinel_command" "$PWD" "$#" "$*" >>"${REALM_SENTINEL_LOG:?}"
exit 97
