# Copyright 2021-2024, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_BUILD_PDF_PAGES_TROFF_INCLUDED
MAKEFILE_BUILD_PDF_PAGES_TROFF_INCLUDED := 1


include $(MAKEFILEDIR)/build/_.mk
include $(MAKEFILEDIR)/build/man/man.mk
include $(MAKEFILEDIR)/build/man/mdoc.mk
include $(MAKEFILEDIR)/configure/build-depends/grep/grep.mk
include $(MAKEFILEDIR)/configure/build-depends/groff-base/troff.mk
include $(MAKEFILEDIR)/configure/xfail.mk


_XFAIL_PDFMAN_MAN_set := \
	$(_MANDIR)/man1/iconv.1.pdf.set \
	$(_MANDIR)/man2/fanotify_init.2.pdf.set \
	$(_MANDIR)/man2/membarrier.2.pdf.set \
	$(_MANDIR)/man2/prctl.2.pdf.set \
	$(_MANDIR)/man2/statx.2.pdf.set \
	$(_MANDIR)/man2/syscall.2.pdf.set \
	$(_MANDIR)/man3/newlocale.3.pdf.set \
	$(_MANDIR)/man7/address_families.7.pdf.set \
	$(_MANDIR)/man7/armscii-8.7.pdf.set \
	$(_MANDIR)/man7/ascii.7.pdf.set \
	$(_MANDIR)/man7/bpf-helpers.7.pdf.set \
	$(_MANDIR)/man7/charsets.7.pdf.set \
	$(_MANDIR)/man7/cp1251.7.pdf.set \
	$(_MANDIR)/man7/iso_8859-2.7.pdf.set \
	$(_MANDIR)/man7/iso_8859-3.7.pdf.set \
	$(_MANDIR)/man7/iso_8859-4.7.pdf.set \
	$(_MANDIR)/man7/iso_8859-5.7.pdf.set \
	$(_MANDIR)/man7/iso_8859-6.7.pdf.set \
	$(_MANDIR)/man7/iso_8859-7.7.pdf.set \
	$(_MANDIR)/man7/iso_8859-8.7.pdf.set \
	$(_MANDIR)/man7/iso_8859-10.7.pdf.set \
	$(_MANDIR)/man7/iso_8859-11.7.pdf.set \
	$(_MANDIR)/man7/iso_8859-13.7.pdf.set \
	$(_MANDIR)/man7/iso_8859-14.7.pdf.set \
	$(_MANDIR)/man7/iso_8859-16.7.pdf.set \
	$(_MANDIR)/man7/koi8-r.7.pdf.set \
	$(_MANDIR)/man7/koi8-u.7.pdf.set \
	$(_MANDIR)/man7/pathname.7.pdf.set \
	$(_MANDIR)/man7/vdso.7.pdf.set


_PDFMAN_MAN_set  := $(patsubst %, %.pdf.set, $(_NONSO_MAN))
_PDFMAN_MDOC_set := $(patsubst %, %.pdf.set, $(_NONSO_MDOC))


ifeq ($(SKIP_XFAIL),yes)
_PDFMAN_MAN_set := $(filter-out $(_XFAIL_PDFMAN_MAN_set), $(_PDFMAN_MAN_set))
endif


$(_PDFMAN_MAN_set): %.pdf.set: %.pdf.troff $(MK) | $$(@D)/
	$(info	$(INFO_)TROFF		$@)
	! ($(TROFF) -man -Tpdf $(TROFFFLAGS_) <$< 2>&1 >$@) \
	| $(GREP) ^ >&2

$(_PDFMAN_MDOC_set): %.pdf.set: %.pdf.troff $(MK) | $$(@D)/
	$(info	$(INFO_)TROFF		$@)
	! ($(TROFF) -mdoc -Tpdf $(TROFFFLAGS_) <$< 2>&1 >$@) \
	| $(GREP) ^ >&2


.PHONY: build-pdf-pages-troff-man
build-pdf-pages-troff-man: $(_PDFMAN_MAN_set);

.PHONY: build-pdf-pages-troff-mdoc
build-pdf-pages-troff-mdoc: $(_PDFMAN_MDOC_set);

.PHONY: build-pdf-pages-troff
build-pdf-pages-troff: build-pdf-pages-troff-man build-pdf-pages-troff-mdoc;


endif  # include guard
