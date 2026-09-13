# Copyright 2021-2024, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_BUILD_HTML_TROFF_INCLUDED
MAKEFILE_BUILD_HTML_TROFF_INCLUDED := 1


include $(MAKEFILEDIR)/build/_.mk
include $(MAKEFILEDIR)/build/man/man.mk
include $(MAKEFILEDIR)/build/man/mdoc.mk
include $(MAKEFILEDIR)/configure/build-depends/grep/grep.mk
include $(MAKEFILEDIR)/configure/build-depends/groff-base/troff.mk
include $(MAKEFILEDIR)/configure/xfail.mk


_XFAIL_HTMLMAN_MAN_set := \
	$(_MANDIR)/man2/fanotify_init.2.html.set \
	$(_MANDIR)/man2/s390_sthyi.2.html.set \
	$(_MANDIR)/man2/mbind.2.html.set \
	$(_MANDIR)/man2/membarrier.2.html.set \
	$(_MANDIR)/man2/set_mempolicy.2.html.set \
	$(_MANDIR)/man4/lirc.4.html.set \
	$(_MANDIR)/man5/proc.5.html.set \
	$(_MANDIR)/man7/address_families.7.html.set \
	$(_MANDIR)/man7/bpf-helpers.7.html.set \
	$(_MANDIR)/man7/charsets.7.html.set \
	$(_MANDIR)/man7/iso_8859-16.7.html.set \
	$(_MANDIR)/man7/iso_8859-6.7.html.set \
	$(_MANDIR)/man8/zic.8.html.set


_HTMLMAN_MAN_set  := $(patsubst %, %.html.set, $(_NONSO_MAN))
_HTMLMAN_MDOC_set := $(patsubst %, %.html.set, $(_NONSO_MDOC))


ifeq ($(SKIP_XFAIL),yes)
_HTMLMAN_MAN_set := $(filter-out $(_XFAIL_HTMLMAN_MAN_set), $(_HTMLMAN_MAN_set))
endif


$(_HTMLMAN_MAN_set): %.html.set: %.eqn $(MK) | $$(@D)/
	$(info	$(INFO_)TROFF		$@)
	! ($(TROFF) -man -Thtml $(TROFFFLAGS_) <$< 2>&1 >$@) \
	| $(GREP) ^ >&2

$(_HTMLMAN_MDOC_set): %.html.set: %.eqn $(MK) | $$(@D)/
	$(info	$(INFO_)TROFF		$@)
	! ($(TROFF) -mdoc -Thtml $(TROFFFLAGS_) <$< 2>&1 >$@) \
	| $(GREP) ^ >&2


.PHONY: build-html-troff-man
build-html-troff-man: $(_HTMLMAN_MAN_set);

.PHONY: build-html-troff-mdoc
build-html-troff-mdoc: $(_HTMLMAN_MDOC_set);

.PHONY: build-html-troff
build-html-troff: build-html-troff-man build-html-troff-mdoc;


endif  # include guard
