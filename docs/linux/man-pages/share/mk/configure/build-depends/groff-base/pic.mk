# Copyright 2024-2025, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_CONFIGURE_BUILD_DEPENDS_GROFF_BASE_PIC_INCLUDED
MAKEFILE_CONFIGURE_BUILD_DEPENDS_GROFF_BASE_PIC_INCLUDED := 1


DEFAULT_PICFLAGS :=
ifndef PICFLAGS
PICFLAGS         :=
endif
ifndef PICFLAGS_
PICFLAGS_        := $(DEFAULT_PICFLAGS) $(PICFLAGS)
endif
ifndef PIC
PIC              := pic
endif


endif  # include guard
