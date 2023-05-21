#!/bin/sh
UNAME=$(uname)
tag="$(grep '"version":' ./package.json | cut -d'"' -f4)"
echo "downloading hyperlink $tag for $UNAME"
case $UNAME in
    Linux) (
        curl -L -o scripts/hyperlink-bin https://github.com/untitaker/hyperlink/releases/download/$tag/hyperlink-linux-x86_64
    ) ;;
    Darwin) (
        curl -L -o scripts/hyperlink-bin https://github.com/untitaker/hyperlink/releases/download/$tag/hyperlink-mac-x86_64
    ) ;;
    CYGWIN*) (
        curl -L -o scripts/hyperlink-bin https://github.com/untitaker/hyperlink/releases/download/$tag/hyperlink-windows-x86_64.exe
    ) ;;
    *) (
        echo "$UNAME not supported"
        exit 1
    )
esac

chmod +x scripts/hyperlink-bin
