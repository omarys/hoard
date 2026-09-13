#! /usr/bin/env bash

set -euo pipefail

cat << EOF

|_  _  _  _ _|
| |(_)(_|| (_|

command organizer

hoard setup
https://github.com/hyde46/hoard

Please create an issue if you encounter any problems!

===============================================================================

EOF

__hoard_install_with_cargo(){
    echo "Building from source..."

    ## Check if cargo is installed
    if ! command -v cargo &> /dev/null
    then
        if command -v rustup &> /dev/null
        then
            echo "rustup was found, but cargo wasn't. Something is up with your installation"
            exit 1
        fi

        echo "cargo not found, installing rustup..."
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -q
        # shellcheck disable=SC1091
        . "$HOME/.cargo/env"
    fi

    echo "Installing hoard from the current checkout..."
    cargo install --path .
}

__hoard_install_ubuntu(){
	echo "Assuming Ubuntu distro. Trying to install .deb package"
	VERSION="$(grep '^version' Cargo.toml | head -1 | cut -d '"' -f2)"
	ARTIFACT_URL="https://github.com/hyde46/hoard/releases/download/v${VERSION}/hoard-rs_${VERSION}_amd64.deb"

	if ! wget -q -O /tmp/hoard.deb "$ARTIFACT_URL"; then
		echo "No .deb package found for this release (${ARTIFACT_URL})."
		echo "Try the 'From Source with cargo' option instead."
		exit 1
	fi

	sudo dpkg -i /tmp/hoard.deb
	rm -f /tmp/hoard.deb
}

__hoard_install_mac(){
    echo "MacOS detected."
    echo "Not yet supported by installer.sh"
    echo "please install hoard from source with cargo or with brew"
    echo "brew tap add Hyde46/hoard"
    echo "brew install hoard"
    echo "To install as zsh plugin, run:"
    echo 'echo "eval \"\$(hoard shell-config -s zsh)\"" >> ~/.zshrc'
}

__hoard_detect_os(){

    case "${OSTYPE:-}" in
        linux*) __hoard_install_ubuntu ;;
        darwin*) __hoard_install_mac ;;
        *) echo "Unsupported OS: ${OSTYPE:-unknown}" ;;
    esac
}

PS3='How do you wish to install hoard:  '
options=("From Source with cargo" "OS-Specific" "Not at all, bye")
select choice in "${options[@]}"; do
    case $choice in
        "From Source with cargo")
            __hoard_install_with_cargo
            break
            ;;
        "OS-Specific")
            __hoard_detect_os
            break
            ;;
	"Not at all, bye")
        echo "Bye!"
	    break
	    ;;
        *) echo "invalid option $REPLY";;
    esac
done

# Register the hoard shell plugin (Ctrl-H opens the interactive UI) for the
# detected login shell. Idempotent: nothing is appended a second time.
case "${SHELL:-}" in
    *zsh*)
        SHELL_RC="$HOME/.zshrc"
        PLUGIN_LINE='eval "$(hoard shell-config -s zsh)"'
        ;;
    *bash*)
        SHELL_RC="$HOME/.bashrc"
        PLUGIN_LINE='eval "$(hoard shell-config -s bash)"'
        ;;
    *fish*)
        SHELL_RC="$HOME/.config/fish/config.fish"
        PLUGIN_LINE='hoard shell-config -s fish | source'
        ;;
    *)
        echo "Could not detect your login shell. To enable the interactive UI, add"
        echo 'the output of `hoard shell-config -s <bash|zsh|fish>` to your shell rc.'
        exit 0
        ;;
esac

if [ ! -f "$SHELL_RC" ]; then
    mkdir -p "$(dirname "$SHELL_RC")"
    touch "$SHELL_RC"
fi

if ! grep -q "hoard shell-config" "$SHELL_RC"; then
    echo "$PLUGIN_LINE" >> "$SHELL_RC"
fi

echo "Done! Source your ${SHELL_RC} and press <Ctrl-H> to get started with the interactive hoard UI"
