# Packaging

## Why the PKGBUILD is not in the repository root

`makepkg` unpacks sources into `src/` and stages the package in `pkg/`,
relative to wherever it is run. A Rust crate keeps its code in `src/`. Run
`makepkg` in the repository root and those two collide: the tarball is
extracted on top of the crate, and `makepkg -c` then deletes the lot on its way
out. The crate's sources are in git so nothing is lost permanently, but you get
a working tree with no code in it and a confusing error about a manifest with
no targets.

Keeping the PKGBUILD in its own directory removes the collision.

## Building the package

```sh
cd packaging/aur
updpkgsums                 # after a version bump; needs pacman-contrib
makepkg -si
namcap PKGBUILD
namcap thinkpoint-*.pkg.tar.zst
```

## Publishing to the AUR

The AUR repository is separate and holds only two files.

```sh
git clone ssh://aur@aur.archlinux.org/thinkpoint.git aur-thinkpoint
cd aur-thinkpoint
cp ../ThinkPoint/packaging/aur/PKGBUILD .
makepkg --printsrcinfo > .SRCINFO
git add PKGBUILD .SRCINFO
git commit -m "thinkpoint 0.2.0"
git push
```

Generate `.SRCINFO` after copying the PKGBUILD, never before: it has to
describe the file being pushed, and the AUR rejects a push where the two
disagree. That is the most common reason a submission bounces.

## On a version bump

1. `pkgver` in the PKGBUILD, and `pkgrel` back to 1.
2. Push the new tag to GitHub so the source tarball exists.
3. `updpkgsums` for the new checksum.
4. Rebuild, then regenerate `.SRCINFO` in the AUR clone.
