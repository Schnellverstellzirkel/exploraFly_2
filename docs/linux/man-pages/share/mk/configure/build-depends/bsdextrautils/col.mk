# Copyright 2024-2025, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_CONFIGURE_BUILD_DEPENDS_BSDEXTRAUTILS_COL_INCLUDED
MAKEFILE_CONFIGURE_BUILD_DEPENDS_BSDEXTRAUTILS_COL_INCLUDED := 1


DEFAULT_COLFLAGS := \
	-b \
	-p \
	-x
ifndef COLFLAGS
COLFLAGS         :=
endif
ifndef COLFLAGS_
COLFLAGS_        := $(DEFAULT_COLFLAGS) $(COLFLAGS)
endif
ifndef COL
COL              := col
endif


endif  # include guard
