# Copyright 2023-2024, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_BUILD_PDF_BOOK_INCLUDED
MAKEFILE_BUILD_PDF_BOOK_INCLUDED := 1


include $(MAKEFILEDIR)/build/_.mk
include $(MAKEFILEDIR)/build/man/_.mk
include $(MAKEFILEDIR)/build/fonts/_.mk
include $(MAKEFILEDIR)/build/fonts/tinos/_.mk
include $(MAKEFILEDIR)/configure/build-depends/coreutils/cat.mk
include $(MAKEFILEDIR)/configure/build-depends/groff/gropdf.mk
include $(MAKEFILEDIR)/configure/build-depends/groff-base/eqn.mk
include $(MAKEFILEDIR)/configure/build-depends/groff-base/pic.mk
include $(MAKEFILEDIR)/configure/build-depends/groff-base/preconv.mk
include $(MAKEFILEDIR)/configure/build-depends/groff-base/tbl.mk
include $(MAKEFILEDIR)/configure/build-depends/groff-base/troff.mk
include $(MAKEFILEDIR)/configure/build-depends/moreutils/sponge.mk
include $(MAKEFILEDIR)/configure/version.mk


MKBOOKDIR := $(MAKEFILEDIR)/build/pdf/book
MKBOOK    := $(wildcard $(MKBOOKDIR)/*)


PDF_BOOK  := $(DISTNAME).pdf
_PDFDIR   := $(builddir)
_PDF_BOOK := $(_PDFDIR)/$(PDF_BOOK)


$(_PDF_BOOK): $(_MANPAGES) $(_DOWNLOAD) $(_TINOS) $(MKBOOK) $(MK) | $$(@D)/
	$(info	$(INFO_)GROPDF		$@)
	$(MKBOOKDIR)/prepare.pl $(_MANDIR) \
	| $(CAT) $(MKBOOKDIR)/front.roff /dev/stdin \
	| $(PRECONV) \
	| $(PIC) \
	| $(TBL) \
	| $(EQN) -Tpdf \
	| $(TROFF) -mandoc -Tpdf -F$(_FONTSDIR) -dpaper=a4 -rC1 \
		-ran*bookmark-base-level=1 $(TROFFFLAGS_) \
	| $(GROPDF) -F$(_FONTSDIR) -pa4 $(GROPDFFLAGS_) \
	| $(SPONGE) $@


.PHONY: build-pdf-book
build-pdf-book: $(_PDF_BOOK);


endif  # include guard
