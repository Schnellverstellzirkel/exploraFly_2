# Copyright 2021-2024, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_BUILD_HTML_POST_GROHTML_INCLUDED
MAKEFILE_BUILD_HTML_POST_GROHTML_INCLUDED := 1


include $(MAKEFILEDIR)/build/html/troff.mk
include $(MAKEFILEDIR)/configure/build-depends/groff/post-grohtml.mk


_HTMLMAN := $(patsubst %.html.set, %.html, $(_HTMLMAN_MAN_set) $(_HTMLMAN_MDOC_set))


$(_HTMLMAN): %.html: %.html.set $(MK) | $$(@D)/
	$(info	$(INFO_)POST_GROHTML	$@)
	$(POST_GROHTML) $(POST_GROHTMLFLAGS_) <$< >$@


.PHONY: build-html-post-grohtml
build-html-post-grohtml: $(_HTMLMAN);


endif  # include guard
