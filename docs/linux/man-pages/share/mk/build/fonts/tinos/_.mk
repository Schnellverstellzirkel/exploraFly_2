# Copyright 2021-2024, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_BUILD_FONTS_TINOS_INCLUDED
MAKEFILE_BUILD_FONTS_TINOS_INCLUDED := 1


include $(MAKEFILEDIR)/build/fonts/tinos/pfa.mk
include $(MAKEFILEDIR)/build/fonts/tinos/dit.mk


_TINOS := $(_TINOS_PFA) $(_TINOSR)


.PHONY: build-fonts-tinos
build-fonts-tinos: $(_TINOS)


endif  # include guard
