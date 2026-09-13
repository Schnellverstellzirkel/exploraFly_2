# Copyright 2021-2024, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_INSTALL_MAN_INCLUDED
MAKEFILE_INSTALL_MAN_INCLUDED := 1


include $(MAKEFILEDIR)/configure/build-depends/coreutils/cat.mk
include $(MAKEFILEDIR)/configure/build-depends/coreutils/install.mk
include $(MAKEFILEDIR)/configure/build-depends/coreutils/ln.mk
include $(MAKEFILEDIR)/configure/build-depends/coreutils/test.mk
include $(MAKEFILEDIR)/configure/build-depends/findutils/xargs.mk
include $(MAKEFILEDIR)/configure/build-depends/grep/grep.mk
include $(MAKEFILEDIR)/configure/build-depends/moreutils/sponge.mk
include $(MAKEFILEDIR)/configure/build-depends/sed/sed.mk
include $(MAKEFILEDIR)/configure/directory_variables/install.mk
include $(MAKEFILEDIR)/configure/directory_variables/src.mk
include $(MAKEFILEDIR)/configure/man/link_pages.mk
include $(MAKEFILEDIR)/install/_.mk
include $(MAKEFILEDIR)/src/man.mk


_mandir := $(DESTDIR)$(mandir)
$(foreach s, $(MANSECTIONS),                                                  \
	$(eval _man$(s)dir := $(DESTDIR)$(man$(s)dir)))

$(foreach s, $(MANSECTIONS),                                                  \
	$(eval _man$(s)pages :=                                               \
		$(patsubst $(MAN$(s)DIR)/%.$(s), $(_man$(s)dir)/%$(man$(s)ext), \
			$(MAN$(s)PAGES))))
$(foreach s, $(MANSECTIONS),                                                  \
	$(eval _man$(s)intropage :=                                           \
		$(patsubst $(MAN$(s)DIR)/%.$(s), $(_man$(s)dir)/%$(man$(s)ext), \
			$(MAN$(s)INTROPAGE))))
_manintropages := $(foreach s, $(MANSECTIONS), $(_man$(s)intropage))
_manpages := $(_manintropages) $(foreach s, $(MANSECTIONS), $(_man$(s)pages))

_manintropages_rm := $(addsuffix -rm, $(wildcard $(_manintropages)))
$(foreach s, $(MANSECTIONS),                                                  \
	$(eval _man$(s)pages_rm :=                                            \
		$(addsuffix -rm,                                              \
			$(wildcard $(_man$(s)pages)))))


$(foreach s, $(MANSECTIONS),                                                  \
	$(eval $(_man$(s)pages) $(_man$(s)intropage):                         \
		$(_man$(s)dir)/%$(man$(s)ext):                                \
			$(_MANDIR)/man$(s)/%.$(s) $(MK) | $$$$(@D)/))


$(_manpages):
	$(info	$(INFO_)INSTALL		$@)
	$(INSTALL_DATA) -T $< $@
	$(SED) -i $(foreach s, $(MANSECTIONS), \
		-e '/^\.so /s, man$(s)/\(.*\)\.$(s)$$, $(notdir $(man$(s)dir))/\1$(man$(s)ext),') \
		$@
ifeq ($(LINK_PAGES),symlink)
	$(CAT) <$@ \
	| if $(GREP) '^\.so ' >/dev/null; then \
		$(GREP) '^\.so ' <$@ \
		| $(SED) 's,^\.so \(.*\),../\1,' \
		| $(XARGS) -I tgt $(LN) -fsT tgt $@; \
	fi
endif


.PHONY: install-manintro
install-manintro:  $(_manintropages);
$(foreach s, $(MANSECTIONS),                                                  \
	$(eval .PHONY: install-man$(s)))
$(foreach s, $(MANSECTIONS),                                                  \
	$(eval install-man$(s): $(_man$(s)pages);))
.PHONY: install-man
install-man: install-manintro $(foreach s, $(MANSECTIONS), install-man$(s));

.PHONY: uninstall-manintro
uninstall-manintro: $(_manintropages_rm);
$(foreach s, $(MANSECTIONS),                                                  \
	$(eval .PHONY: uninstall-man$(s)))
$(foreach s, $(MANSECTIONS),                                                  \
	$(eval uninstall-man$(s): $(_man$(s)pages_rm);))
.PHONY: uninstall-man
uninstall-man: uninstall-manintro $(foreach s, $(MANSECTIONS), uninstall-man$(s));


endif  # include guard
