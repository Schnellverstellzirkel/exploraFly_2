# Copyright 2021-2024, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_BUILD_FONTS_TINOS_PFA_INCLUDED
MAKEFILE_BUILD_FONTS_TINOS_PFA_INCLUDED := 1


include $(MAKEFILEDIR)/build/fonts/_.mk
include $(MAKEFILEDIR)/configure/build-depends/texlive-fonts-extra/Tinos.pfb.mk
include $(MAKEFILEDIR)/configure/build-depends/groff/pfbtops.mk


_TINOS_PFA := $(_FONTSDIR)/devpdf/Tinos.pfa


$(_TINOS_PFA): $(TINOS_PFB) $(MK) | $$(@D)/
	$(info	$(INFO_)PFBTOPS		$@)
	$(PFBTOPS) <$< >$@


endif  # include guard
