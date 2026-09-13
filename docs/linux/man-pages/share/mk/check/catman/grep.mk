# Copyright 2021-2024, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_CHECK_CATMAN_GREP_INCLUDED
MAKEFILE_CHECK_CATMAN_GREP_INCLUDED := 1


include $(MAKEFILEDIR)/build/_.mk
include $(MAKEFILEDIR)/configure/build-depends/coreutils/touch.mk
include $(MAKEFILEDIR)/configure/build-depends/grep/grep.mk
include $(MAKEFILEDIR)/configure/build-depends/man/man.mk
include $(MAKEFILEDIR)/configure/xfail.mk


_XFAIL_CHECK_catman := \
	$(_MANDIR)/man1/memusage.1.check-catman.touch \
	$(_MANDIR)/man3/mallopt.3.check-catman.touch \
	$(_MANDIR)/man4/smartpqi.4.check-catman.touch \
	$(_MANDIR)/man4/veth.4.check-catman.touch \
	$(_MANDIR)/man5/proc_buddyinfo.5.check-catman.touch \
	$(_MANDIR)/man5/proc_pid_fdinfo.5.check-catman.touch \
	$(_MANDIR)/man5/proc_pid_maps.5.check-catman.touch \
	$(_MANDIR)/man5/proc_pid_mountinfo.5.check-catman.touch \
	$(_MANDIR)/man5/proc_pid_net.5.check-catman.touch \
	$(_MANDIR)/man5/proc_timer_stats.5.check-catman.touch \
	$(_MANDIR)/man5/proc_version.5.check-catman.touch \
	$(_MANDIR)/man5/slabinfo.5.check-catman.touch \
	$(_MANDIR)/man7/keyrings.7.check-catman.touch \
	$(_MANDIR)/man7/string_copying.7.check-catman.touch \
	$(_MANDIR)/man7/uri.7.check-catman.touch


_CHECK_catman := $(patsubst %.cat.grep, %.check-catman.touch, $(_CHECK_catman_grep))
ifeq ($(SKIP_XFAIL),yes)
_CHECK_catman := $(filter-out $(_XFAIL_CHECK_catman), $(_CHECK_catman))
endif


$(_CHECK_catman): %.check-catman.touch: %.cat.grep $(MK) | $$(@D)/
	$(info	$(INFO_)GREP		$@)
	! $(GREP) -n '.\{$(MANWIDTH)\}.' $< /dev/null >&2
	$(TOUCH) $@


.PHONY: check-catman-grep
check-catman-grep: $(_CHECK_catman);


endif  # include guard
