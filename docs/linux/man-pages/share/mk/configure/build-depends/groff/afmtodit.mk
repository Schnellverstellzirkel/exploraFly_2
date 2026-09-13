# Copyright 2024-2025, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_CONFIGURE_BUILD_DEPENDS_GROFF_AFMTODIT_INCLUDED
MAKEFILE_CONFIGURE_BUILD_DEPENDS_GROFF_AFMTODIT_INCLUDED := 1


ifndef PDF_TEXT_ENC
PDF_TEXT_ENC := /usr/share/groff/current/font/devpdf/enc/text.enc
endif
ifndef PDF_TEXT_MAP
PDF_TEXT_MAP := /usr/share/groff/current/font/devpdf/map/text.map
endif


DEFAULT_AFMTODITFLAGS :=
ifndef AFMTODITFLAGS
AFMTODITFLAGS         :=
endif
ifndef AFMTODITFLAGS_
AFMTODITFLAGS_        := $(DEFAULT_AFMTODITFLAGS) $(AFMTODITFLAGS)
endif
ifndef AFMTODIT
AFMTODIT              := afmtodit
endif


endif  # include guard
