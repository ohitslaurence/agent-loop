#!/bin/bash
set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo "Installing loop..."

# Check for claude CLI
if ! command -v claude &> /dev/null; then
    echo -e "${YELLOW}Warning: Claude CLI not found - loop requires it to run${NC}"
    echo "Install from: https://docs.anthropic.com/en/docs/claude-code"
fi

# Check for gum (optional)
GUM_MISSING=false
if ! command -v gum &> /dev/null; then
    GUM_MISSING=true
fi

# Determine install location
INSTALL_DIR="$HOME/.local/bin"
if [[ "$1" == "--global" ]] || [[ "$1" == "-g" ]]; then
    INSTALL_DIR="/usr/local/bin"
fi

# Create install directory if needed
if [[ "$INSTALL_DIR" == "$HOME/.local/bin" ]]; then
    mkdir -p "$INSTALL_DIR"
fi

# Get script directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Make scripts executable
chmod +x "$SCRIPT_DIR/bin/loop"
chmod +x "$SCRIPT_DIR/bin/loop-analyze"

# Create symlinks
if [[ "$INSTALL_DIR" == "/usr/local/bin" ]]; then
    echo "Installing to $INSTALL_DIR (requires sudo)..."
    if ! sudo ln -sf "$SCRIPT_DIR/bin/loop" "$INSTALL_DIR/loop"; then
        echo -e "${RED}Error: Failed to create symlink (permission denied?)${NC}"
        exit 1
    fi
    if ! sudo ln -sf "$SCRIPT_DIR/bin/loop-analyze" "$INSTALL_DIR/loop-analyze"; then
        echo -e "${RED}Error: Failed to create symlink for analyze script${NC}"
        exit 1
    fi
else
    echo "Installing to $INSTALL_DIR..."
    if ! ln -sf "$SCRIPT_DIR/bin/loop" "$INSTALL_DIR/loop"; then
        echo -e "${RED}Error: Failed to create symlink${NC}"
        exit 1
    fi
    if ! ln -sf "$SCRIPT_DIR/bin/loop-analyze" "$INSTALL_DIR/loop-analyze"; then
        echo -e "${RED}Error: Failed to create symlink for analyze script${NC}"
        exit 1
    fi
fi

# Check if install dir is in PATH
if [[ ":$PATH:" != *":$INSTALL_DIR:"* ]]; then
    echo -e "${YELLOW}Warning: $INSTALL_DIR is not in your PATH${NC}"
    echo ""
    # Detect shell config file
    SHELL_NAME="$(basename "$SHELL")"
    SHELL_CONFIG=""
    case "$SHELL_NAME" in
        zsh)  SHELL_CONFIG="$HOME/.zshrc" ;;
        bash) SHELL_CONFIG="$HOME/.bashrc" ;;
        fish) SHELL_CONFIG="$HOME/.config/fish/config.fish" ;;
    esac

    if [[ -n "$SHELL_CONFIG" ]]; then
        echo "Run this command to add to your PATH:"
        echo -e "  ${GREEN}touch $SHELL_CONFIG && echo 'export PATH=\"$INSTALL_DIR:\$PATH\"' >> $SHELL_CONFIG && source $SHELL_CONFIG${NC}"
    else
        echo "Add this to your shell config:"
        echo -e "  ${GREEN}export PATH=\"$INSTALL_DIR:\$PATH\"${NC}"
    fi
    echo ""
fi

echo -e "${GREEN}loop installed successfully!${NC}"
echo ""

# Warn about missing gum
if [[ "$GUM_MISSING" == "true" ]]; then
    echo -e "${YELLOW}Note: gum not found - interactive spec picker won't work${NC}"
    echo "  Install: brew install gum  OR  go install github.com/charmbracelet/gum@latest"
    echo "  Or run with --no-gum for plain output"
    echo ""
fi

echo "Usage:"
echo "  loop specs/my-feature.md"
echo "  loop --init-config    # Create project config"
echo "  loop --help           # Show all options"
