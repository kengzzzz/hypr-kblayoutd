# AUR packages

| Package | Directory | Installs |
| --- | --- | --- |
| `hypr-kblayoutd-bin` | `aur-bin/` | the GitHub release tarball |
| `hypr-kblayoutd-git` | `aur-git/` | latest `main`, built from source |

Both provide and conflict with `hypr-kblayoutd`. These copies are the source of truth: edit here, then copy to the AUR checkouts.

One-time setup (needs an AUR SSH key), from the repo root:

```sh
git clone ssh://aur@aur.archlinux.org/hypr-kblayoutd-bin.git build/aur-bin
git clone ssh://aur@aur.archlinux.org/hypr-kblayoutd-git.git build/aur-git
```

## Release `-bin`

After the Release workflow has published the `v<version>` tarballs:

```sh
packaging/update-bin-pkgbuild.sh 0.3.2
git commit -m "release checksum for AUR" packaging/aur-bin

git -C build/aur-bin pull
cp packaging/aur-bin/PKGBUILD packaging/aur-bin/.SRCINFO build/aur-bin/
cd build/aur-bin
makepkg --verifysource --force
git add PKGBUILD .SRCINFO
git commit -m "upgpkg: 0.3.2-1"
git push origin master
```

## Update `-git`

`pkgver()` computes the version at build time, so push only when the PKGBUILD changes. To refresh the version AUR shows, build once so `makepkg` rewrites `pkgver`:

```sh
git -C build/aur-git pull
cp packaging/aur-git/PKGBUILD build/aur-git/
cd build/aur-git
makepkg --force --cleanbuild
makepkg --printsrcinfo > .SRCINFO
cp PKGBUILD .SRCINFO ../../packaging/aur-git/
git add PKGBUILD .SRCINFO
git commit -m "upgpkg: $(sed -n 's/^pkgver=//p' PKGBUILD)-1"
git push origin master
```
