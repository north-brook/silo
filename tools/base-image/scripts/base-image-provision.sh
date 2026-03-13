#!/usr/bin/env bash
set -euxo pipefail

readonly MARKER_SUCCESS="SILO_BASE_IMAGE_PROVISIONING_COMPLETE"
readonly MARKER_FAILURE="SILO_BASE_IMAGE_PROVISIONING_FAILED"
readonly DEVELOPER_USER="silo"
readonly DEVELOPER_HOME="/home/${DEVELOPER_USER}"
readonly BREW_HOME="/home/linuxbrew"
readonly BREW_PREFIX="${BREW_HOME}/.linuxbrew"
readonly BUN_INSTALL_DIR="${DEVELOPER_HOME}/.bun"

serial_log() {
  local message="$1"
  echo "$message"
  if [[ -w /dev/ttyS0 ]]; then
    echo "$message" > /dev/ttyS0
  fi
}

run_as_developer() {
  local command="$1"
  su - "${DEVELOPER_USER}" -s /bin/bash -c "export HOME='${DEVELOPER_HOME}'; ${command}"
}

on_error() {
  local exit_code=$?
  serial_log "${MARKER_FAILURE}:${exit_code}"
  exit "${exit_code}"
}

trap on_error ERR

export DEBIAN_FRONTEND=noninteractive
export HOME=/root

serial_log "Starting silo base image provisioning"

apt-get update
apt-get install -y \
  alsa-utils \
  build-essential \
  ca-certificates \
  cmake \
  coreutils \
  curl \
  direnv \
  fd-find \
  file \
  findutils \
  git \
  gnupg \
  glib-networking \
  jq \
  jackd2 \
  less \
  libdrm2 \
  libegl1 \
  libgcrypt20 \
  libgirepository-1.0-1 \
  libgl1 \
  libglib2.0-0 \
  libgles1 \
  libgles2 \
  libglvnd0 \
  libglx0 \
  libgudev-1.0-0 \
  libjack-jackd2-0 \
  libopengl0 \
  libopus0 \
  libpulse0 \
  libvpx-dev \
  libwayland-dev \
  libwayland-egl1 \
  libx11-xcb1 \
  libxcb-dri3-0 \
  libxdamage1 \
  libxext6 \
  libxfixes3 \
  libxtst6 \
  libxv1 \
  make \
  patch \
  procps \
  psmisc \
  pulseaudio \
  python3 \
  python3-dev \
  python3-gi \
  python3-pip \
  python3-setuptools \
  python3-venv \
  python3-wheel \
  rsync \
  ripgrep \
  shellcheck \
  sqlite3 \
  sudo \
  tmux \
  tree \
  unzip \
  wayland-protocols \
  wget \
  wmctrl \
  x11-utils \
  x11-xkb-utils \
  x11-xserver-utils \
  x264 \
  x265 \
  xdotool \
  xfce4 \
  xfce4-terminal \
  xsel \
  xserver-xorg-core \
  xvfb \
  xz-utils \
  zip \
  zsh

serial_log "Installing selkies-gstreamer runtime dependencies"

apt-get install -y \
  aom-tools \
  libopenh264-dev \
  svt-av1 \
  xcvt || true

SELKIES_VERSION="$(
  curl -fsSL "https://api.github.com/repos/selkies-project/selkies/releases/latest" \
    | jq -r '.tag_name' \
    | sed 's/[^0-9.\-]*//g'
)"
DISTRIB_RELEASE="$(. /etc/os-release && printf '%s' "${VERSION_ID}")"

install -d -m 0755 /opt
rm -rf /opt/gstreamer /opt/gst-web

curl -fsSL \
  "https://github.com/selkies-project/selkies/releases/download/v${SELKIES_VERSION}/gstreamer-selkies_gpl_v${SELKIES_VERSION}_ubuntu${DISTRIB_RELEASE}_amd64.tar.gz" \
  | tar -C /opt -xzf -

curl -fsSLo "/tmp/selkies_gstreamer-${SELKIES_VERSION}-py3-none-any.whl" \
  "https://github.com/selkies-project/selkies/releases/download/v${SELKIES_VERSION}/selkies_gstreamer-${SELKIES_VERSION}-py3-none-any.whl"
PIP_BREAK_SYSTEM_PACKAGES=1 pip3 install --no-cache-dir --ignore-installed \
  "/tmp/selkies_gstreamer-${SELKIES_VERSION}-py3-none-any.whl"
rm -f "/tmp/selkies_gstreamer-${SELKIES_VERSION}-py3-none-any.whl"

curl -fsSL \
  "https://github.com/selkies-project/selkies/releases/download/v${SELKIES_VERSION}/selkies-gstreamer-web_v${SELKIES_VERSION}.tar.gz" \
  | tar -C /opt -xzf -

printf '%s\n' "${SELKIES_VERSION}" > /opt/selkies-version

if ! id -u "${DEVELOPER_USER}" >/dev/null 2>&1; then
  useradd -m -d "${DEVELOPER_HOME}" -s /usr/bin/zsh "${DEVELOPER_USER}"
fi

usermod -aG sudo "${DEVELOPER_USER}"
install -d -m 0750 -o "${DEVELOPER_USER}" -g "${DEVELOPER_USER}" "${DEVELOPER_HOME}"
install -d -m 0750 /etc/sudoers.d
cat > /etc/sudoers.d/silo <<EOF
${DEVELOPER_USER} ALL=(ALL) NOPASSWD:ALL
EOF
chmod 0440 /etc/sudoers.d/silo

install -d -m 0755 -o "${DEVELOPER_USER}" -g "${DEVELOPER_USER}" "${BREW_HOME}"

if [[ ! -x "${BREW_PREFIX}/bin/brew" ]]; then
  run_as_developer 'NONINTERACTIVE=1 /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"'
fi

eval "$("${BREW_PREFIX}/bin/brew" shellenv)"
export PATH="${BREW_PREFIX}/bin:${BREW_PREFIX}/sbin:${PATH}"

run_as_developer "eval \"\$(${BREW_PREFIX}/bin/brew shellenv)\" && brew tap neurosnap/tap"
run_as_developer "eval \"\$(${BREW_PREFIX}/bin/brew shellenv)\" && brew install gh just node@22 rust yq zig neurosnap/tap/zmx"
run_as_developer "eval \"\$(${BREW_PREFIX}/bin/brew shellenv)\" && brew link --overwrite --force node@22"

if [[ ! -x "${BUN_INSTALL_DIR}/bin/bun" ]]; then
  run_as_developer 'curl -fsSL https://bun.sh/install | bash'
fi

export PATH="${BUN_INSTALL_DIR}/bin:${PATH}"

run_as_developer "export PATH='${BUN_INSTALL_DIR}/bin:${BREW_PREFIX}/bin:${BREW_PREFIX}/sbin:\$PATH' && corepack enable"
run_as_developer "export PATH='${BUN_INSTALL_DIR}/bin:${BREW_PREFIX}/bin:${BREW_PREFIX}/sbin:\$PATH' && corepack prepare yarn@stable --activate"
run_as_developer "export PATH='${BUN_INSTALL_DIR}/bin:${BREW_PREFIX}/bin:${BREW_PREFIX}/sbin:\$PATH' && corepack prepare pnpm@latest --activate"

run_as_developer "export PATH='${BUN_INSTALL_DIR}/bin:${BREW_PREFIX}/bin:${BREW_PREFIX}/sbin:\$PATH' && npm install -g @anthropic-ai/claude-code @openai/codex"

mkdir -p /etc/profile.d
cat > /etc/profile.d/silo-homebrew.sh <<'EOF'
export HOMEBREW_PREFIX="/home/linuxbrew/.linuxbrew"
export BUN_INSTALL="/home/silo/.bun"
export SELKIES_VERSION_FILE="/opt/selkies-version"
export SELKIES_GSTREAMER_ROOT="/opt/gstreamer"
export SELKIES_WEB_ROOT="/opt/gst-web"

if [ -x "${HOMEBREW_PREFIX}/bin/brew" ]; then
  eval "$("${HOMEBREW_PREFIX}/bin/brew" shellenv)"
fi

if [ -d "${BUN_INSTALL}/bin" ]; then
  export PATH="${BUN_INSTALL}/bin:${PATH}"
fi

export PATH="/usr/local/bin:${PATH}"
EOF

cat > /etc/profile.d/silo-zsh.sh <<'EOF'
if [ -n "${ZSH_VERSION:-}" ]; then
  return 0 2>/dev/null || exit 0
fi

case $- in
  *i*) ;;
  *)
    return 0 2>/dev/null || exit 0
    ;;
esac

if command -v zsh >/dev/null 2>&1; then
  exec zsh -l
fi
EOF

install -d -m 0750 -o "${DEVELOPER_USER}" -g "${DEVELOPER_USER}" "${DEVELOPER_HOME}"

if [[ ! -d "${DEVELOPER_HOME}/.oh-my-zsh" ]]; then
  git clone --depth=1 https://github.com/ohmyzsh/ohmyzsh.git "${DEVELOPER_HOME}/.oh-my-zsh"
fi

cp "${DEVELOPER_HOME}/.oh-my-zsh/templates/zshrc.zsh-template" "${DEVELOPER_HOME}/.zshrc"
sed -i 's|^export ZSH=.*|export ZSH="$HOME/.oh-my-zsh"|' "${DEVELOPER_HOME}/.zshrc"
sed -i 's/^ZSH_THEME=.*/ZSH_THEME="robbyrussell"/' "${DEVELOPER_HOME}/.zshrc"
grep -qxF 'PROMPT_EOL_MARK=""' "${DEVELOPER_HOME}/.zshrc" || \
  printf 'PROMPT_EOL_MARK=""\n' >> "${DEVELOPER_HOME}/.zshrc"
grep -qxF 'export PATH="/usr/local/bin:$PATH"' "${DEVELOPER_HOME}/.zshrc" || \
  printf '\nexport PATH="/usr/local/bin:$PATH"\n' >> "${DEVELOPER_HOME}/.zshrc"
grep -qxF '[[ -f /etc/profile.d/silo-homebrew.sh ]] && source /etc/profile.d/silo-homebrew.sh' "${DEVELOPER_HOME}/.zshrc" || \
  printf '[[ -f /etc/profile.d/silo-homebrew.sh ]] && source /etc/profile.d/silo-homebrew.sh\n' >> "${DEVELOPER_HOME}/.zshrc"
if ! grep -qxF 'if [[ $- == *i* ]] && [[ -t 0 ]] && [[ -t 1 ]]; then' "${DEVELOPER_HOME}/.zshrc"; then
  cat >> "${DEVELOPER_HOME}/.zshrc" <<'EOF'
if [[ $- == *i* ]] && [[ -t 0 ]] && [[ -t 1 ]]; then
  # Normalize terminal line discipline for interactive shells.
  stty sane 2>/dev/null || true
  stty erase '^?' -ixon -ixoff icrnl -inlcr -igncr opost onlcr isig icanon iexten echo echoe echok echoctl 2>/dev/null || true

  # Match common terminal editing shortcuts to zsh widgets.
  bindkey '^A' beginning-of-line
  bindkey '^E' end-of-line
  bindkey '^U' backward-kill-line
  bindkey '^[b' backward-word
  bindkey '^[f' forward-word
  bindkey '^[[1;3D' backward-word
  bindkey '^[[1;3C' forward-word

  # Match xterm.js capabilities for Silo-managed interactive SSH terminals.
  export TERM=xterm-256color
  export COLORTERM="${COLORTERM:-truecolor}"
fi
EOF
fi
chown -R "${DEVELOPER_USER}:${DEVELOPER_USER}" "${DEVELOPER_HOME}/.oh-my-zsh" "${DEVELOPER_HOME}/.zshrc"

if command -v fdfind >/dev/null 2>&1; then
  ln -sf "$(command -v fdfind)" /usr/local/bin/fd
fi

for command_name in bun cargo claude codex corepack direnv gh just node npm npx pnpm rustc shellcheck yarn zig zmx; do
  target_path="$(command -v "${command_name}" || true)"
  if [[ -n "${target_path}" ]]; then
    ln -sf "${target_path}" "/usr/local/bin/${command_name}"
  fi
done

rm -f /usr/local/bin/brew
ln -sf /usr/bin/python3 /usr/local/bin/python3
if [[ -x /usr/bin/pip3 ]]; then
  ln -sf /usr/bin/pip3 /usr/local/bin/pip3
fi
export PATH="/usr/local/bin:${PATH}"

for command_name in curl file find git jq less make patch pgrep rg rsync sqlite3 tmux tree wget yq; do
  target_path="$(command -v "${command_name}" || true)"
  if [[ -n "${target_path}" ]]; then
    ln -sf "${target_path}" "/usr/local/bin/${command_name}"
  fi
done

"${BREW_PREFIX}/bin/brew" --version
cmake --version
curl --version
direnv version
fdfind --version
find --version
git --version
gh --version
jq --version
less --version
make --version
node --version
patch --version
pgrep --version
python3 --version
npm --version
yarn --version
pnpm --version
bun --version
command -v codex
command -v claude
cargo --version
rustc --version
just --version
rg --version
rsync --version
shellcheck --version
sqlite3 --version
tmux -V
tree --version
wget --version
yq --version
zig version
command -v zmx
zsh --version
id "${DEVELOPER_USER}"
cat /opt/selkies-version
python3 -c 'import selkies_gstreamer'
test -d /opt/gstreamer
test -d /opt/gst-web
su - "${DEVELOPER_USER}" -s /bin/bash -c 'for command_name in brew bun cargo claude codex curl fd git gh jq node npm pnpm python3 rg rustc yarn yq zig zmx zsh; do command -v "${command_name}" >/dev/null; done'
su - "${DEVELOPER_USER}" -s /bin/bash -c 'test -f /opt/selkies-version && python3 -c "import selkies_gstreamer"'
su - "${DEVELOPER_USER}" -s /bin/bash -c '
  prefix="$(brew --prefix)"
  repo="$(brew --repository)"
  cache_dir="$(brew --cache)"
  install -d "${prefix}/Cellar" "${cache_dir}"
  touch "${prefix}/.silo-brew-write-test" "${prefix}/Cellar/.silo-brew-write-test" "${repo}/.silo-brew-write-test" "${cache_dir}/.silo-brew-write-test"
  rm -f "${prefix}/.silo-brew-write-test" "${prefix}/Cellar/.silo-brew-write-test" "${repo}/.silo-brew-write-test" "${cache_dir}/.silo-brew-write-test"
'
su - "${DEVELOPER_USER}" -s /bin/bash -c 'sudo -n true'

serial_log "${MARKER_SUCCESS}"
