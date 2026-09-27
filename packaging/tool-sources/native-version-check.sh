#!/bin/sh
# Version identity is the first line; callers retain the full diagnostic output.
starship_version_is_selected() {
    case $1 in
        'starship 1.23.0'|'starship 1.23.0
'*) return 0 ;;
        *) return 1 ;;
    esac
}
