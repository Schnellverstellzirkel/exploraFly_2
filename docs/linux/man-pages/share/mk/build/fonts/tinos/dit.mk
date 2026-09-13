# Copyright 2021-2024, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_BUILD_FONTS_TINOS_DIT_INCLUDED
MAKEFILE_BUILD_FONTS_TINOS_DIT_INCLUDED := 1


include $(MAKEFILEDIR)/build/fonts/_.mk
include $(MAKEFILEDIR)/build/fonts/tinos/afm.mk
include $(MAKEFILEDIR)/configure/build-depends/groff/afmtodit.mk


_TINOSR := $(_FONTSDIR)/devpdf/TinosR


$(_TINOSR): $(_FONTSDIR)/%: $(PDF_TEXT_ENC) $(_TINOSR_AFM) $(PDF_TEXT_MAP) $(MK) | $$(@D)/
	$(info	$(INFO_)AFMTODIT	$@)
	$(AFMTODIT) -e $(PDF_TEXT_ENC) $(_TINOSR_AFM) $(PDF_TEXT_MAP) $@


endif  # include guard
