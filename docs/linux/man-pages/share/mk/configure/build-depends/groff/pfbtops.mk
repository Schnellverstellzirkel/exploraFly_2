# Copyright 2024-2025, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_CONFIGURE_BUILD_DEPENDS_GROFF_PFBTOPS_INCLUDED
MAKEFILE_CONFIGURE_BUILD_DEPENDS_GROFF_PFBTOPS_INCLUDED := 1


DEFAULT_PFBTOPSFLAGS :=
ifndef PFBTOPSFLAGS
PFBTOPSFLAGS         :=
endif
ifndef PFBTOPSFLAGS_
PFBTOPSFLAGS_        := $(DEFAULT_PFBTOPSFLAGS) $(PFBTOPSFLAGS)
endif
ifndef PFBTOPS
PFBTOPS              := pfbtops
endif


endif  # include guard
