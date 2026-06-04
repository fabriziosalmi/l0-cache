#!/bin/bash
# ==============================================================================
# 'l0-cache' - CLI Proxy Setup Script
# A beautiful, resilient installer for macOS and Linux.
# ==============================================================================

set -euo pipefail

# --- Color Definitions & Icons ---
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BLUE='\033[0;34m'
MAGENTA='\033[0;35m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m' # No Color

ICON_INFO="${BLUE}●${NC}"
ICON_SUCCESS="${GREEN}●${NC}"
ICON_WARNING="${YELLOW}●${NC}"
ICON_ERROR="${RED}●${NC}"
ICON_GEAR="${CYAN}●${NC}"

# --- Configuration Paths ---
LOCAL_BIN_DIR="$HOME/.local/bin"
GLOBAL_BIN_DIR="/usr/local/bin"
BINARY_NAME="l0-cache"
DRY_RUN=false
BUILD_LOG="target/install-build.log"

# --- Functions ---

# Banner printing
print_banner() {
    echo -e "${BOLD}${CYAN}"
    echo "  ┌────────────────────────────────────────────────────────┐"
    echo "  │               l0-cache  CLI Proxy Setup                │"
    echo "  │         Universal LLM Token Savings Pipeline           │"
    echo "  └────────────────────────────────────────────────────────┘"
    echo -e "${NC}"
}

# Logger helpers
log_info() { echo -e "  ${ICON_INFO}  $*"; }
log_success() { echo -e "  ${ICON_SUCCESS}  ${GREEN}$*${NC}"; }
log_warn() { echo -e "  ${ICON_WARNING}  ${YELLOW}$*${NC}"; }
log_err() { echo -e "  ${ICON_ERROR}  ${RED}$*${NC}"; }

show_help() {
    print_banner
    echo "Usage:"
    echo "  ./install.sh                  Build and install 'l0-cache' (interactive)"
    echo "  ./install.sh --local          Install locally to $LOCAL_BIN_DIR"
    echo "  ./install.sh --global         Install globally to $GLOBAL_BIN_DIR (requires sudo)"
    echo "  ./install.sh --install-hook   Install Git pre-commit quality hook in the repository"
    echo "  ./install.sh --uninstall-hook Remove Git pre-commit quality hook"
    echo "  ./install.sh --uninstall      Uninstall 'l0-cache' from the system"
    echo "  ./install.sh --dry-run        Show planned installation steps without making changes"
    echo "  ./install.sh --help           Show this help documentation"
    exit 0
}

# Detect system properties
detect_system() {
    local osarch
    osarch=$(uname -m)
    local osname
    osname=$(uname -s)
    echo -e "  System: ${BOLD}${osname} (${osarch})${NC}"
}

detect_shell() {
    local shell_name
    shell_name=$(basename "$SHELL")
    echo "$shell_name"
}

# Spinner logic for background processes
run_with_spinner() {
    local pid=$1
    local message=$2
    local spinstr='⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏'
    
    # Hide cursor
    if [ -t 1 ] && [ -n "${TERM:-}" ] && type tput >/dev/null 2>&1; then tput civis 2>/dev/null || true; fi
    
    while kill -0 "$pid" 2>/dev/null; do
        for (( i=0; i<${#spinstr}; i++ )); do
            printf "\r  ${CYAN}%c${NC}  %s" "${spinstr:$i:1}" "$message"
            sleep 0.08
        done
    done
    
    # Restore cursor
    if [ -t 1 ] && [ -n "${TERM:-}" ] && type tput >/dev/null 2>&1; then tput cnorm 2>/dev/null || true; fi
    printf "\r" # Reset carriage
}

# Uninstall flow
uninstall() {
    echo -e "\n${BOLD}${YELLOW}🧹 Initiating Uninstallation...${NC}\n"
    local removed=0

    # Local binary removal
    if [ -f "$LOCAL_BIN_DIR/$BINARY_NAME" ]; then
        if [ "$DRY_RUN" = true ]; then
            log_info "[Dry-Run] Would remove local binary: $LOCAL_BIN_DIR/$BINARY_NAME"
            log_info "[Dry-Run] Would remove local symlink: $LOCAL_BIN_DIR/t"
        else
            rm "$LOCAL_BIN_DIR/$BINARY_NAME"
            if [ -f "$LOCAL_BIN_DIR/t" ]; then rm "$LOCAL_BIN_DIR/t"; fi
            log_success "Removed local binary and short command 't'."
        fi
        removed=1
    fi

    # Global binary removal
    if [ -f "$GLOBAL_BIN_DIR/$BINARY_NAME" ]; then
        if [ "$DRY_RUN" = true ]; then
            log_info "[Dry-Run] Would remove global binary: $GLOBAL_BIN_DIR/$BINARY_NAME"
            log_info "[Dry-Run] Would remove global symlink: $GLOBAL_BIN_DIR/t"
        else
            if [ -w "$GLOBAL_BIN_DIR" ]; then
                rm "$GLOBAL_BIN_DIR/$BINARY_NAME"
                if [ -f "$GLOBAL_BIN_DIR/t" ]; then rm "$GLOBAL_BIN_DIR/t"; fi
            else
                log_warn "Requesting sudo privileges to delete global files..."
                sudo rm "$GLOBAL_BIN_DIR/$BINARY_NAME"
                if [ -f "$GLOBAL_BIN_DIR/t" ]; then sudo rm "$GLOBAL_BIN_DIR/t"; fi
            fi
            log_success "Removed global binary and short command 't'."
        fi
        removed=1
    fi

    # Completion removals
    # Zsh
    if [ -f "$HOME/.zfunc/_$BINARY_NAME" ]; then
        if [ "$DRY_RUN" = true ]; then
            log_info "[Dry-Run] Would remove zsh completion: $HOME/.zfunc/_$BINARY_NAME"
            log_info "[Dry-Run] Would remove zsh completion: $HOME/.zfunc/_t"
        else
            rm "$HOME/.zfunc/_$BINARY_NAME"
            if [ -f "$HOME/.zfunc/_t" ]; then rm "$HOME/.zfunc/_t"; fi
            log_success "Removed zsh completions."
        fi
        removed=1
    fi
    # Fish
    if [ -f "$HOME/.config/fish/completions/$BINARY_NAME.fish" ]; then
        if [ "$DRY_RUN" = true ]; then
            log_info "[Dry-Run] Would remove fish completion: $HOME/.config/fish/completions/$BINARY_NAME.fish"
            log_info "[Dry-Run] Would remove fish completion: $HOME/.config/fish/completions/t.fish"
        else
            rm "$HOME/.config/fish/completions/$BINARY_NAME.fish"
            if [ -f "$HOME/.config/fish/completions/t.fish" ]; then rm "$HOME/.config/fish/completions/t.fish"; fi
            log_success "Removed fish completions."
        fi
        removed=1
    fi
    # Bash
    if [ -f "$HOME/.local/share/bash-completion/completions/$BINARY_NAME" ]; then
        if [ "$DRY_RUN" = true ]; then
            log_info "[Dry-Run] Would remove bash completion: $HOME/.local/share/bash-completion/completions/$BINARY_NAME"
            log_info "[Dry-Run] Would remove bash completion: $HOME/.local/share/bash-completion/completions/t"
        else
            rm "$HOME/.local/share/bash-completion/completions/$BINARY_NAME"
            if [ -f "$HOME/.local/share/bash-completion/completions/t" ]; then rm "$HOME/.local/share/bash-completion/completions/t"; fi
            log_success "Removed bash completions."
        fi
        removed=1
    fi

    if [ "$DRY_RUN" = true ]; then
        log_success "Dry-run uninstall check completed successfully."
    elif [ $removed -eq 1 ]; then
        log_success "Uninstall completed. 'l0-cache' is no longer present on your system."
    else
        log_warn "No installations of 'l0-cache' were found in typical directories."
    fi
    exit 0
}

# Perform installation
perform_install() {
    local target_dir=$1
    local is_global=$2

    echo -e "\n${BOLD}${CYAN}● Launching Installation Flow...${NC}\n"
    detect_system

    # Check for cargo/rust toolchain
    if ! command -v cargo &> /dev/null; then
        log_err "Rust toolchain ('cargo') not found!"
        echo "  Please install Rust before proceeding: https://rustup.rs/"
        echo "  Or run: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
        exit 1
    fi

    # 1. Compile release binary
    if [ "$DRY_RUN" = true ]; then
        log_info "[Dry-Run] Would build 'l0-cache' in release mode using: cargo build --release"
    else
        mkdir -p target
        echo -e "  ${ICON_GEAR}  Compiling 'l0-cache'..."
        cargo build --release > "$BUILD_LOG" 2>&1 &
        local cargo_pid=$!
        
        run_with_spinner "$cargo_pid" "Compiling 'l0-cache' release binary (this may take a moment)..."
        
        if wait "$cargo_pid"; then
            local size
            size=$(du -h target/release/$BINARY_NAME | cut -f1)
            log_success "Compilation succeeded! Release binary size: $size"
        else
            log_err "Compilation failed! Check build logs: $BUILD_LOG"
            echo "--------------------------------------------------------"
            tail -n 20 "$BUILD_LOG"
            echo "--------------------------------------------------------"
            exit 1
        fi
    fi

    # 2. Deploy binary
    local dest_path="$target_dir/$BINARY_NAME"
    if [ "$DRY_RUN" = true ]; then
        log_info "[Dry-Run] Would copy 'l0-cache' to $dest_path"
        log_info "[Dry-Run] Would create short command 't' symlink in $target_dir"
    else
        log_info "Deploying binary to $dest_path..."
        if [ "$is_global" = true ]; then
            if [ -w "$target_dir" ]; then
                cp "target/release/$BINARY_NAME" "$dest_path"
                ln -sf "$BINARY_NAME" "$target_dir/t"
            else
                log_warn "Destination requires root permissions. Prompting sudo..."
                sudo cp "target/release/$BINARY_NAME" "$dest_path"
                sudo chmod +x "$dest_path"
                sudo ln -sf "$BINARY_NAME" "$target_dir/t"
            fi
        else
            mkdir -p "$target_dir"
            cp "target/release/$BINARY_NAME" "$dest_path"
            chmod +x "$dest_path"
            ln -sf "$BINARY_NAME" "$target_dir/t"
        fi
        log_success "Deployment completed successfully (including short command 't')."
    fi

    # 3. PATH configuration check
    if [ "$is_global" = false ]; then
        local current_shell
        current_shell=$(detect_shell)
        local profile_file=""
        
        if [ "$current_shell" = "zsh" ]; then
            profile_file="$HOME/.zshrc"
        elif [ "$current_shell" = "bash" ]; then
            profile_file="$HOME/.bashrc"
            if [ ! -f "$profile_file" ] && [ -f "$HOME/.bash_profile" ]; then
                profile_file="$HOME/.bash_profile"
            fi
        fi

        if [ "$current_shell" = "fish" ]; then
            if [ "$DRY_RUN" = true ]; then
                log_info "[Dry-Run] Would configure fish path: fish -c \"fish_add_path $target_dir\""
            else
                if ! fish -c "echo \$PATH" 2>/dev/null | grep -q "$target_dir"; then
                    log_info "Configuring fish shell path..."
                    fish -c "fish_add_path $target_dir"
                    log_success "Fish path configured."
                fi
            fi
        elif [ -n "$profile_file" ] && [ -f "$profile_file" ]; then
            if ! grep -q "$target_dir" "$profile_file"; then
                if [ "$DRY_RUN" = true ]; then
                    log_info "[Dry-Run] Would append $target_dir to PATH in $profile_file"
                else
                    log_info "Adding $target_dir to PATH in $profile_file..."
                    echo "" >> "$profile_file"
                    echo "# Added by 'l0-cache' installer" >> "$profile_file"
                    echo "export PATH=\"\$PATH:$target_dir\"" >> "$profile_file"
                    log_success "PATH configured. Please restart your terminal or run: source $profile_file"
                fi
            else
                log_info "$target_dir is already in your shell configuration PATH."
            fi
        else
            log_warn "Unknown shell or configuration file. Please manually verify that $target_dir is in your \$PATH."
        fi
    else
        # Global install: verify the destination is actually reachable on $PATH.
        if echo ":$PATH:" | grep -q ":$target_dir:"; then
            log_info "$target_dir is on your \$PATH."
        else
            log_warn "$target_dir is not on your \$PATH — 'l0-cache' may not be found."
            log_warn "Add it to your shell profile, e.g.: export PATH=\"\$PATH:$target_dir\""
        fi
    fi

    # 4. Completions setup
    local check_bin="target/release/$BINARY_NAME"
    if [ "$DRY_RUN" = true ]; then
        log_info "[Dry-Run] Would generate shell completions for zsh, bash, and fish"
    else
        setup_completions "$check_bin"
    fi

    # 5. Print Integration Guide
    if [ "$DRY_RUN" = false ]; then
        print_integration_info "$dest_path"
    else
        log_success "Dry-run check completed successfully."
    fi
}

setup_completions() {
    local bin_path=$1
    local current_shell
    current_shell=$(detect_shell)

    log_info "Setting up auto-completion profiles..."

    # Zsh
    local zsh_completion_dir="$HOME/.zfunc"
    mkdir -p "$zsh_completion_dir"
    "$bin_path" --completions zsh > "$zsh_completion_dir/_l0-cache"
    cat << 'EOF' > "$zsh_completion_dir/_t"
#compdef t
_l0-cache "$@"
EOF
    
    local zshrc="$HOME/.zshrc"
    if [ -f "$zshrc" ] && ! grep -q "fpath+=($zsh_completion_dir)" "$zshrc"; then
        log_info "Configuring zsh completion paths in $zshrc..."
        sed -i.bak "1s|^|fpath+=($zsh_completion_dir)\nautoload -Uz compinit\ncompinit\n|" "$zshrc" 2>/dev/null || \
        echo -e "fpath+=($zsh_completion_dir)\nautoload -Uz compinit\ncompinit\n$(cat "$zshrc")" > "$zshrc"
    fi
    log_success "Zsh completions generated in $zsh_completion_dir/_l0-cache and _t"

    # Fish
    if command -v fish &>/dev/null; then
        local fish_completion_dir="$HOME/.config/fish/completions"
        mkdir -p "$fish_completion_dir"
        "$bin_path" --completions fish > "$fish_completion_dir/l0-cache.fish"
        echo "complete -c t -w l0-cache" > "$fish_completion_dir/t.fish"
        log_success "Fish completions generated in $fish_completion_dir/l0-cache.fish and t.fish"
    fi

    # Bash
    local bash_completion_dir="$HOME/.local/share/bash-completion/completions"
    mkdir -p "$bash_completion_dir"
    "$bin_path" --completions bash > "$bash_completion_dir/l0-cache"
    cat << 'EOF' > "$bash_completion_dir/t"
if [ -f "$HOME/.local/share/bash-completion/completions/l0-cache" ]; then
    . "$HOME/.local/share/bash-completion/completions/l0-cache"
fi
complete -F _l0-cache t
EOF
    log_success "Bash completions generated in $bash_completion_dir/l0-cache and t"
}

print_integration_info() {
    local abs_path=$1
    echo -e "\n${BOLD}${BLUE}======================================================================${NC}"
    echo -e "${GREEN}● 'l0-cache' is installed and ready to use!${NC}"
    echo -e "Location: ${BOLD}${YELLOW}$abs_path${NC}"
    echo -e "${BOLD}${BLUE}======================================================================${NC}\n"
    
    echo -e "${BOLD}${CYAN}Integration Guide for your AI / Coding tools:${NC}\n"
    
    echo -e "1. ${BOLD}${GREEN}VS Code (Terminal)${NC}"
    echo -e "   Once installed, 'l0-cache' is available in your VS Code integrated terminal."
    echo -e "   Prefix your commands: ${YELLOW}l0-cache cargo test${NC} or ${YELLOW}l0-cache git diff${NC}."
    echo ""
    
    echo -e "2. ${BOLD}${GREEN}Claude Code (claudecode CLI)${NC}"
    echo -e "   Claude Code inherits your shell's environment PATH."
    echo -e "   Tell Claude Code to run commands prefixed with 'l0-cache':"
    echo -e "   ${YELLOW}\"run: l0-cache cargo test\"${NC} or ${YELLOW}\"run: l0-cache npm test\"${NC}"
    echo -e "   This will automatically save tokens by filtering and collapsing redundant outputs."
    echo ""
    
    echo -e "3. ${BOLD}${GREEN}Claude Desktop (macOS App)${NC}"
    echo -e "   macOS GUI apps launched from Finder do not read shell profiles (~/.zshrc)."
    echo -e "   To run tools under Claude Desktop using 'l0-cache', configure your MCP tool"
    echo -e "   definition in ${YELLOW}~/Library/Application Support/Claude/claude_desktop_config.json${NC}."
    echo -e "   Reference the absolute path to 'l0-cache':"
    echo -e "   ${YELLOW}\"$abs_path\"${NC} instead of \"l0-cache\"."
    echo ""
    
    echo -e "4. ${BOLD}${GREEN}Claude CLI (Command Line)${NC}"
    echo -e "   The CLI tool operates directly in your shell."
    echo -e "   Prefix commands with 'l0-cache' to keep Claude's context clean and save tokens."
    echo -e "${BOLD}${BLUE}======================================================================${NC}\n"
}

# Setup Git quality and security pre-commit hook
setup_git_hook() {
    if [ ! -d ".git" ]; then
        log_err "Not a git repository. Cannot install pre-commit hook."
        exit 1
    fi
    log_info "Installing git pre-commit quality gate..."
    mkdir -p .git/hooks

    # Write pre-commit hook file directly
    cat << 'EOF' > .git/hooks/pre-commit
#!/bin/bash
# ==============================================================================
# l0-cache - Git Pre-Commit Hook
# ==============================================================================
set -euo pipefail

# Print banner
echo -e "\033[1m\033[36m● Gating Quality & Security: Running l0-cache Pre-Commit Checks...\033[0m"

# 1. Rustfmt check
echo -e "\n\033[1m● [1/3] Checking code formatting...\033[0m"
if ! cargo fmt -- --check; then
    echo -e "\033[31m● Formatting check failed! Run 'cargo fmt' to resolve.\033[0m"
    exit 1
fi
echo -e "\033[32m● Formatting is clean.\033[0m"

# 2. Clippy lint check
echo -e "\n\033[1m● [2/3] Running Clippy lints...\033[0m"
if ! cargo clippy --all-targets -- -D warnings; then
    echo -e "\033[31m● Clippy lints failed! Fix warnings to proceed.\033[0m"
    exit 1
fi
echo -e "\033[32m● Clippy checks passed.\033[0m"

# 3. Test suite
echo -e "\n\033[1m● [3/3] Running full test suite...\033[0m"
if ! cargo test; then
    echo -e "\033[31m● Tests failed! Fix failing tests to proceed.\033[0m"
    exit 1
fi
echo -e "\033[32m● All tests passed.\033[0m"

# 4. Optional Security Audit
if command -v cargo-audit &>/dev/null; then
    echo -e "\n\033[1m● [Bonus] Running Dependency Security Audit...\033[0m"
    if ! cargo audit; then
        echo -e "\033[31m● Vulnerable dependencies found! Update Cargo.lock to proceed.\033[0m"
        exit 1
    fi
    echo -e "\033[32m● Security audit passed.\033[0m"
else
    echo -e "\n\033[33m● [Skip] cargo-audit is not installed. Skipping dependency security checks.\033[0m"
    echo -e "   Install it with: cargo install cargo-audit"
fi

echo -e "\n\033[1m\033[32m● All checks passed! Committing changes.\033[0m"
exit 0
EOF

    chmod +x .git/hooks/pre-commit
    log_success "Git pre-commit quality hook installed successfully at .git/hooks/pre-commit"
}

# Check if we are inside the l0-cache repository
is_in_repo() {
    if [ -f "Cargo.toml" ] && grep -q 'name = "l0-cache"' Cargo.toml; then
        return 0
    fi
    return 1
}

# --- Argument Parsing ---
ARGS=("$@")

# If not running in the repo, clone it to a temp dir first
if ! is_in_repo; then
    log_info "Not running inside the l0-cache repository. Cloning from GitHub..."
    
    # Check if git is installed
    if ! command -v git &>/dev/null; then
        log_err "Git is required to clone and build l0-cache. Please install git."
        exit 1
    fi
    
    TEMP_DIR=$(mktemp -d -t l0-cache-install-XXXXXXXX)
    git clone --quiet https://github.com/fabriziosalmi/l0-cache.git "$TEMP_DIR"

    # Run the installer from the cloned repo
    cd "$TEMP_DIR"

    # Pin to the latest RELEASE TAG rather than building whatever is at the tip of
    # the default branch. Reduces the blast radius of a compromised/forced-push
    # repo: an unattended `curl | bash` builds a released revision, not arbitrary
    # HEAD. (There are no signed binaries yet — see SECURITY.md.)
    LATEST_TAG=$(git describe --tags --abbrev=0 2>/dev/null || true)
    if [ -n "$LATEST_TAG" ]; then
        log_info "Pinning to latest release tag: ${BOLD}$LATEST_TAG${NC}"
        git checkout --quiet "$LATEST_TAG"
    else
        log_warn "No release tag found; building from the default-branch tip."
    fi
    # Pass all original arguments to the sub-installer.
    # NOTE: "${ARGS[@]+"${ARGS[@]}"}" expands to ZERO words when ARGS is empty.
    # The older "${ARGS[@]:-}" form injected a single empty-string argument, which
    # fell through to the unknown-argument case and printed help instead of
    # installing — breaking the advertised `curl ... | bash` one-liner.
    bash ./install.sh "${ARGS[@]+"${ARGS[@]}"}"
    
    # Clean up temp dir
    cd - >/dev/null
    rm -rf "$TEMP_DIR"
    exit 0
fi

while [[ $# -gt 0 ]]; do
    case $1 in
        --help|-h)
            show_help
            ;;
        --dry-run)
            DRY_RUN=true
            shift
            ;;
        --uninstall)
            uninstall
            ;;
        --local)
            perform_install "$LOCAL_BIN_DIR" false
            exit 0
            ;;
        --global)
            perform_install "$GLOBAL_BIN_DIR" true
            exit 0
            ;;
        --install-hook)
            setup_git_hook
            exit 0
            ;;
        --uninstall-hook)
            if [ -f ".git/hooks/pre-commit" ]; then
                rm ".git/hooks/pre-commit"
                log_success "Git pre-commit hook removed."
            else
                log_warn "No pre-commit hook found."
            fi
            exit 0
            ;;
        *)
            log_err "Unknown argument: $1"
            show_help
            ;;
    esac
done

# If stdin is not a terminal (e.g. piped via curl | bash), run local install automatically
if [ ! -t 0 ]; then
    log_info "Piped execution detected (non-interactive)."
    log_warn "This will clone l0-cache, compile it with cargo, and install to $LOCAL_BIN_DIR."
    log_warn "Review the script first if you do not trust the source. Proceeding with Local Install..."
    perform_install "$LOCAL_BIN_DIR" false
    exit 0
fi

# Interactive Mode
print_banner
echo "Please select an option:"
echo -e "  1) ${BOLD}Local Install${NC} (to $LOCAL_BIN_DIR, no sudo needed)"
echo -e "  2) ${BOLD}Global Install${NC} (to $GLOBAL_BIN_DIR, requires sudo privileges)"
echo -e "  3) ${BOLD}Install Git Hook${NC} (pre-commit quality & security gate)"
echo -e "  4) ${BOLD}Uninstall${NC} 'l0-cache' from the system"
echo -e "  5) ${BOLD}Exit${NC}"
echo ""
read -r -p "Enter choice [1-5]: " choice

case $choice in
    1)
        perform_install "$LOCAL_BIN_DIR" false
        ;;
    2)
        perform_install "$GLOBAL_BIN_DIR" true
        ;;
    3)
        setup_git_hook
        ;;
    4)
        uninstall
        ;;
    *)
        echo -e "Exiting setup."
        exit 0
        ;;
esac
