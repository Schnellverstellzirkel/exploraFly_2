# Copyright 2021-2025, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_CONFIGURE_BUILD_DEPENDS_TAR_TAR_INCLUDED
MAKEFILE_CONFIGURE_BUILD_DEPENDS_TAR_TAR_INCLUDED := 1


include $(MAKEFILEDIR)/configure/version.mk


DEFAULT_TARFLAGS := \
	--sort=name \
	--owner=root:0 \
	--group=root:0 \
	--mtime='$(DISTDATE)'
ifndef TARFLAGS
TARFLAGS         :=
endif
ifndef TARFLAGS_
TARFLAGS_        := $(DEFAULT_TARFLAGS) $(TARFLAGS)
endif
ifndef TAR
TAR              := tar
endif


endif  # include guard
