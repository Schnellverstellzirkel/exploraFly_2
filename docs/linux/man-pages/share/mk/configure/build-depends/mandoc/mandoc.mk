# Copyright 2024-2025, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_CONFIGURE_BUILD_DEPENDS_MANDOC_MANDOC_INCLUDED
MAKEFILE_CONFIGURE_BUILD_DEPENDS_MANDOC_MANDOC_INCLUDED := 1


DEFAULT_MANDOCFLAGS := -Tlint
ifndef MANDOCFLAGS
MANDOCFLAGS         :=
endif
ifndef MANDOCFLAGS_
MANDOCFLAGS_        := $(DEFAULT_MANDOCFLAGS) $(MANDOCFLAGS)
endif
ifndef MANDOC
MANDOC              := mandoc
endif


endif  # include guard
