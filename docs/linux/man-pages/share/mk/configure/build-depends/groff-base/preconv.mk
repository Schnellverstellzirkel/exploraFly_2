# Copyright 2024-2025, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_CONFIGURE_BUILD_DEPENDS_GROFF_BASE_PRECONV_INCLUDED
MAKEFILE_CONFIGURE_BUILD_DEPENDS_GROFF_BASE_PRECONV_INCLUDED := 1


DEFAULT_PRECONVFLAGS :=
ifndef PRECONVFLAGS
PRECONVFLAGS         :=
endif
ifndef PRECONVFLAGS_
PRECONVFLAGS_        := $(DEFAULT_PRECONVFLAGS) $(PRECONVFLAGS)
endif
ifndef PRECONV
PRECONV              := preconv
endif


endif  # include guard
