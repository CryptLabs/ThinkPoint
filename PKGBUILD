# Maintainer: Al1nuX <cryptlabs@gmail.com>
pkgname=thinkpoint
pkgver=0.1.0
pkgrel=1
pkgdesc="TUI for TrackPoint tuning, pointer button maps and libinput properties"
arch=('x86_64' 'aarch64')
url="https://github.com/CryptLabs/ThinkPoint"
license=('MIT')
depends=('gcc-libs')
optdepends=(
  'xorg-xinput: button maps, libinput properties and the source detector'
  'polkit: write sysfs and udev files without dropping to a shell'
)
makedepends=('cargo')
source=("$pkgname-$pkgver.tar.gz::$url/archive/refs/tags/v$pkgver.tar.gz")
sha256sums=('SKIP')

prepare() {
  cd "$pkgname-$pkgver"
  export RUSTUP_TOOLCHAIN=stable
  cargo fetch --locked --target "$(rustc -vV | sed -n 's/host: //p')"
}

build() {
  cd "$pkgname-$pkgver"
  export RUSTUP_TOOLCHAIN=stable
  export CARGO_TARGET_DIR=target
  cargo build --frozen --release --all-features
}

check() {
  cd "$pkgname-$pkgver"
  export RUSTUP_TOOLCHAIN=stable
  cargo test --frozen --all-features
}

package() {
  cd "$pkgname-$pkgver"
  install -Dm0755 "target/release/$pkgname" "$pkgdir/usr/bin/$pkgname"
  install -Dm0644 README.md "$pkgdir/usr/share/doc/$pkgname/README.md"
  install -Dm0644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
}
