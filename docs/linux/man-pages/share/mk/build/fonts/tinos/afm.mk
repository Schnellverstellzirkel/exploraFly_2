# Copyright 2021-2024, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_BUILD_FONTS_TINOS_AFM_INCLUDED
MAKEFILE_BUILD_FONTS_TINOS_AFM_INCLUDED := 1


include $(MAKEFILEDIR)/build/fonts/_.mk
include $(MAKEFILEDIR)/configure/build-depends/texlive-fonts-extra-links/Tinos-Regular.ttf.mk
include $(MAKEFILEDIR)/configure/build-depends/fontforge/fontforge.mk


_TINOSR_AFM := $(_FONTSDIR)/devpdf/TinosR.afm


$(_TINOSR_AFM): $(TINOSR_TTF) $(MK) | $$(@D)/
	$(info	$(INFO_)FONTFORGE	$@)
	$(FONTFORGE) $(FONTFORGEFLAGS_) -lang=ff -c 'Open("$<");Generate("$@");'


endif  # include guard
