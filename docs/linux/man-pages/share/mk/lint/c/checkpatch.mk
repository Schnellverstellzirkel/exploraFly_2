# Copyright 2021-2024, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_LINT_C_CHECKPATCH_INCLUDED
MAKEFILE_LINT_C_CHECKPATCH_INCLUDED := 1


include $(MAKEFILEDIR)/build/_.mk
include $(MAKEFILEDIR)/build/examples/src.mk
include $(MAKEFILEDIR)/configure/build-depends/checkpatch/checkpatch.mk
include $(MAKEFILEDIR)/configure/build-depends/coreutils/touch.mk
include $(MAKEFILEDIR)/configure/xfail.mk


_XFAIL_LINT_c_EX_checkpatch := $(_MANDIR)/man2/bpf.2.d/bpf.c.lint-c.checkpatch.touch


_LINT_c_EX_checkpatch   := $(patsubst %, %.lint-c.checkpatch.touch, $(_EX_TU_src))
ifeq ($(SKIP_XFAIL),yes)
_LINT_c_EX_checkpatch   := $(filter-out $(_XFAIL_LINT_c_EX_checkpatch), $(_LINT_c_EX_checkpatch))
endif
_LINT_c_checkpatch      := $(_LINT_c_EX_checkpatch)


$(_LINT_c_EX_checkpatch): %.lint-c.checkpatch.touch: %
$(_LINT_c_checkpatch): $(CHECKPATCH_CONF) $(MK) | $$(@D)/


$(_LINT_c_checkpatch):
	$(info	$(INFO_)CHECKPATCH	$@)
	$(CHECKPATCH) $(CHECKPATCHFLAGS_) -f $< >&2
	$(TOUCH) $@


.PHONY: lint-c-checkpatch
lint-c-checkpatch: $(_LINT_c_checkpatch);


endif  # include guard
