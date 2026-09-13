# Copyright 2021-2024, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_INSTALL_HTML_INCLUDED
MAKEFILE_INSTALL_HTML_INCLUDED := 1


include $(MAKEFILEDIR)/build/_.mk
include $(MAKEFILEDIR)/build/html/post-grohtml.mk
include $(MAKEFILEDIR)/configure/build-depends/coreutils/install.mk
include $(MAKEFILEDIR)/configure/directory_variables/install.mk
include $(MAKEFILEDIR)/install/_.mk


_htmldir := $(DESTDIR)$(htmldir)


_htmlpages    := $(patsubst $(_MANDIR)/%, $(_htmldir)/%, $(_HTMLMAN))
_htmlpages_rm := $(addsuffix -rm, $(wildcard $(_htmlpages)))


$(_htmlpages): $(_htmldir)/%: $(_MANDIR)/% $(MK) | $$(@D)/
	$(info	$(INFO_)INSTALL		$@)
	$(INSTALL_DATA) -T $< $@


.PHONY: install-html
install-html: $(_htmlpages);

.PHONY: uninstall-html
uninstall-html: $(_htmlpages_rm);


endif  # include guard
