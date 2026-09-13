# Copyright 2022-2025, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_CONFIGURE_BUILD_DEPENDS_PKGCONF_PKGCONF_INCLUDED
MAKEFILE_CONFIGURE_BUILD_DEPENDS_PKGCONF_PKGCONF_INCLUDED := 1


# Compat
ifndef PKG_CONFIG
PKG_CONFIG := pkgconf
endif


DEFAULT_PKGCONFFLAGS :=
ifndef PKGCONFFLAGS
PKGCONFFLAGS         :=
endif
ifndef PKGCONFFLAGS_
PKGCONFFLAGS_        := $(DEFAULT_PKGCONFFLAGS) $(PKGCONFFLAGS)
endif
ifndef PKGCONF
PKGCONF              := $(PKG_CONFIG)
endif
PKGCONF_CMD          := $(PKGCONF) $(PKGCONFFLAGS_)


PKGCONF_LIBS := libbsd-overlay


endif  # include guard
