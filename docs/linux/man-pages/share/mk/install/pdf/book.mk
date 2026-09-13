# Copyright 2021-2024, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_INSTALL_PDF_BOOK_INCLUDED
MAKEFILE_INSTALL_PDF_BOOK_INCLUDED := 1


include $(MAKEFILEDIR)/build/pdf/book/_.mk
include $(MAKEFILEDIR)/configure/build-depends/coreutils/install.mk
include $(MAKEFILEDIR)/configure/directory_variables/install.mk


_pdfdir := $(DESTDIR)$(pdfdir)


_pdf_book    := $(patsubst $(_PDFDIR)/%, $(_pdfdir)/%, $(_PDF_BOOK))
_pdf_book_rm := $(addsuffix -rm, $(wildcard $(_pdf_book)))


$(_pdf_book): $(_pdfdir)/%: $(_PDFDIR)/% $(MK) | $$(@D)/
	$(info	$(INFO_)INSTALL		$@)
	$(INSTALL_DATA) -T $< $@


.PHONY: install-pdf-book
install-pdf-book: $(_pdf_book);

.PHONY: uninstall-pdf-book
uninstall-pdf-book: $(_pdf_book_rm);


endif  # include guard
