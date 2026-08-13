#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 4 ]]; then
  echo "usage: $0 BINARY VERSION ARCH OUTPUT_DIR" >&2
  exit 2
fi

binary=$(realpath "$1")
version=$2
arch=$3
output_dir=$(realpath -m "$4")
repo_root=$(realpath "$(dirname "$0")/..")

[[ -x "$binary" ]]
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+-rc\.[0-9]+$ ]]
[[ "$arch" == "x86_64" ]]

source_date_epoch=${SOURCE_DATE_EPOCH:-$(git -C "$repo_root" log -1 --format=%ct)}
[[ "$source_date_epoch" =~ ^[0-9]+$ ]]
mkdir -p "$output_dir"
work=$(mktemp -d "${TMPDIR:-/tmp}/amatl-packages-XXXXXXXX")
trap 'rm -rf -- "$work"' EXIT

install_payload() {
  local root=$1
  install -Dpm0755 "$binary" "$root/usr/bin/amatl"
  install -Dpm0644 "$repo_root/README.md" "$root/usr/share/doc/amatl/README.md"
  install -Dpm0644 "$repo_root/LICENSE-MIT" "$root/usr/share/licenses/amatl/LICENSE-MIT"
  install -Dpm0644 "$repo_root/LICENSE-APACHE" "$root/usr/share/licenses/amatl/LICENSE-APACHE"
  install -Dpm0644 "$repo_root/packaging/amatl.1" "$root/usr/share/man/man1/amatl.1"
  install -Dpm0755 "$repo_root/packaging/amatl-chromium-sandbox" \
    "$root/usr/libexec/amatl/amatl-chromium-sandbox"
  find "$root" -exec touch --date="@$source_date_epoch" {} +
}

deb_version=${version/-rc./~rc.}
deb_asset_version=${deb_version//\~/.}
deb_root="$work/deb"
install_payload "$deb_root"
mkdir -p "$deb_root/DEBIAN"
cat >"$deb_root/DEBIAN/control" <<EOF
Package: amatl
Version: $deb_version
Section: utils
Priority: optional
Architecture: amd64
Maintainer: IA-Alex <IA-Alex@users.noreply.github.com>
Homepage: https://github.com/IA-Alex/amatl
Description: Generalist multi-source search
 Fast, modular and failure-tolerant Linux-first search with optional Deep evidence.
EOF
touch --date="@$source_date_epoch" "$deb_root/DEBIAN/control" "$deb_root/DEBIAN"
dpkg-deb --root-owner-group --build "$deb_root" \
  "$output_dir/amatl_${deb_asset_version}_amd64.deb"

arch_root="$work/arch"
install_payload "$arch_root"
arch_version=${version/-rc./_rc.}
cat >"$arch_root/.PKGINFO" <<EOF
pkgname = amatl
pkgbase = amatl
pkgver = $arch_version-1
pkgdesc = Generalist multi-source search
url = https://github.com/IA-Alex/amatl
builddate = $source_date_epoch
packager = IA-Alex <IA-Alex@users.noreply.github.com>
size = $(du -sb "$arch_root" | cut -f1)
arch = x86_64
license = MIT
license = Apache-2.0
EOF
touch --date="@$source_date_epoch" "$arch_root/.PKGINFO"
tar --sort=name --owner=0 --group=0 --numeric-owner --mtime="@$source_date_epoch" \
  --zstd -C "$arch_root" -cf "$output_dir/amatl-${arch_version}-1-x86_64.pkg.tar.zst" .

rpm_version=${version%%-*}
rpm_release=${version#*-}
rpm_release=${rpm_release//-/.}
rpm_top="$work/rpm"
mkdir -p "$rpm_top"/{BUILD,BUILDROOT,RPMS,SOURCES,SPECS,SRPMS}
cat >"$rpm_top/SPECS/amatl.spec" <<'EOF'
Name:           amatl
Version:        %{amatl_version}
Release:        %{amatl_release}%{?dist}
Summary:        Generalist multi-source search
License:        MIT OR Apache-2.0
URL:            https://github.com/IA-Alex/amatl
BuildArch:      x86_64

%description
Fast, modular and failure-tolerant Linux-first search with optional Deep evidence.

%install
install -Dpm0755 %{amatl_binary} %{buildroot}%{_bindir}/amatl
install -Dpm0644 %{amatl_readme} %{buildroot}%{_docdir}/amatl/README.md
install -Dpm0644 %{amatl_mit} %{buildroot}%{_licensedir}/amatl/LICENSE-MIT
install -Dpm0644 %{amatl_apache} %{buildroot}%{_licensedir}/amatl/LICENSE-APACHE
install -Dpm0644 %{amatl_manpage} %{buildroot}%{_mandir}/man1/amatl.1
install -Dpm0755 %{amatl_chromium_sandbox} %{buildroot}%{_libexecdir}/amatl/amatl-chromium-sandbox

%files
%{_bindir}/amatl
%doc %{_docdir}/amatl/README.md
%license %{_licensedir}/amatl/LICENSE-MIT
%license %{_licensedir}/amatl/LICENSE-APACHE
%{_mandir}/man1/amatl.1*
%{_libexecdir}/amatl/amatl-chromium-sandbox

%changelog
* Thu Aug 13 2026 IA-Alex <IA-Alex@users.noreply.github.com> - %{version}-%{release}
- Reproducible AMATL release-candidate package.
EOF
rpmbuild --define "_topdir $rpm_top" \
  --define "amatl_version $rpm_version" \
  --define "amatl_release $rpm_release" \
  --define "amatl_binary $binary" \
  --define "amatl_readme $repo_root/README.md" \
  --define "amatl_mit $repo_root/LICENSE-MIT" \
  --define "amatl_apache $repo_root/LICENSE-APACHE" \
  --define "amatl_manpage $repo_root/packaging/amatl.1" \
  --define "amatl_chromium_sandbox $repo_root/packaging/amatl-chromium-sandbox" \
  --define "source_date_epoch $source_date_epoch" \
  --define "use_source_date_epoch_as_buildtime 1" \
  --define "build_mtime_policy clamp_to_source_date_epoch" \
  --define "_buildhost reproducible.amatl.invalid" \
  -bb "$rpm_top/SPECS/amatl.spec"
find "$rpm_top/RPMS" -type f -name '*.rpm' -exec cp {} "$output_dir/" \;

find "$output_dir" -maxdepth 1 -type f -exec touch --date="@$source_date_epoch" {} +
