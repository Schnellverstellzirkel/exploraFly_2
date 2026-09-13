# Copyright 2021-2024, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_LINT_MAN_TBL_INCLUDED
MAKEFILE_LINT_MAN_TBL_INCLUDED := 1


include $(MAKEFILEDIR)/build/man/man.mk
include $(MAKEFILEDIR)/configure/build-depends/coreutils/cat.mk
include $(MAKEFILEDIR)/configure/build-depends/coreutils/echo.mk
include $(MAKEFILEDIR)/configure/build-depends/coreutils/head.mk
include $(MAKEFILEDIR)/configure/build-depends/coreutils/tail.mk
include $(MAKEFILEDIR)/configure/build-depends/coreutils/touch.mk
include $(MAKEFILEDIR)/configure/build-depends/grep/grep.mk


_LINT_man_tbl := $(patsubst %, %.lint-man.tbl.touch, $(_NONSO_MAN))


$(_LINT_man_tbl): %.lint-man.tbl.touch: % $(MK) | $$(@D)/
	$(info	$(INFO_)GREP		$@)
	$(HEAD) -n1 <$< \
	| if $(GREP) '\\" t$$' >/dev/null; then \
		$(CAT) <$< \
		| if ! $(GREP) '^\.TS$$' >/dev/null; then \
			>&2 $(ECHO) "$<:1: spurious '\\\" t' comment:"; \
			>&2 $(HEAD) -n1 <$<; \
			exit 1; \
		fi; \
	else \
		$(CAT) <$< \
		| if $(GREP) '^\.TS$$' >/dev/null; then \
			>&2 $(ECHO) "$<:1: missing '\\\" t' comment:"; \
			>&2 $(HEAD) -n1 <$<; \
			exit 1; \
		fi; \
	fi
	$(TAIL) -n+2 <$< \
	| if $(GREP) '\\" t$$' >/dev/null; then \
		>&2 $(ECHO) "$<: spurious '\\\" t' not in first line:"; \
		>&2 $(GREP) -n '\\" t$$' $< /dev/null; \
		exit 1; \
	fi
	$(TOUCH) $@


.PHONY: lint-man-tbl
lint-man-tbl: $(_LINT_man_tbl);


endif  # include guard
