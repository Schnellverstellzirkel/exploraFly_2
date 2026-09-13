# Copyright 2024-2025, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_CONFIGURE_BUILD_DEPENDS_GROFF_GROPDF_INCLUDED
MAKEFILE_CONFIGURE_BUILD_DEPENDS_GROFF_GROPDF_INCLUDED := 1


DEFAULT_GROPDFFLAGS :=
ifndef GROPDFFLAGS
GROPDFFLAGS         :=
endif
ifndef GROPDFFLAGS_
GROPDFFLAGS_        := $(DEFAULT_GROPDFFLAGS) $(GROPDFFLAGS)
endif
ifndef GROPDF
GROPDF              := gropdf
endif


endif  # include guard
