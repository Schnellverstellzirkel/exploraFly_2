# Copyright 2021-2024, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_LINT_C_CPPCHECK_INCLUDED
MAKEFILE_LINT_C_CPPCHECK_INCLUDED := 1


include $(MAKEFILEDIR)/build/_.mk
include $(MAKEFILEDIR)/build/examples/src.mk
include $(MAKEFILEDIR)/configure/build-depends/coreutils/touch.mk
include $(MAKEFILEDIR)/configure/build-depends/cppcheck/cppcheck.mk
include $(MAKEFILEDIR)/configure/xfail.mk


_XFAIL_LINT_c_EX_cppcheck := \
	$(_MANDIR)/man2/chown.2.d/chown.c.lint-c.cppcheck.touch \
	$(_MANDIR)/man2/close_range.2.d/close_range.c.lint-c.cppcheck.touch \
	$(_MANDIR)/man2/kcmp.2.d/kcmp.c.lint-c.cppcheck.touch \
	$(_MANDIR)/man2/keyctl.2.d/key_instantiate.c.lint-c.cppcheck.touch \
	$(_MANDIR)/man2/memfd_create.2.d/t_memfd_create.c.lint-c.cppcheck.touch \
	$(_MANDIR)/man2/msgop.2.d/msgop.c.lint-c.cppcheck.touch \
	$(_MANDIR)/man2/open_by_handle_at.2.d/t_open_by_handle_at.c.lint-c.cppcheck.touch \
	$(_MANDIR)/man2/shmop.2.d/svshm_string_read.c.lint-c.cppcheck.touch \
	$(_MANDIR)/man2/unshare.2.d/unshare.c.lint-c.cppcheck.touch \
	$(_MANDIR)/man3/bsearch.3.d/bsearch.c.lint-c.cppcheck.touch \
	$(_MANDIR)/man3/dl_iterate_phdr.3.d/dl_iterate_phdr.c.lint-c.cppcheck.touch \
	$(_MANDIR)/man3/dlopen.3.d/dlopen.c.lint-c.cppcheck.touch \
	$(_MANDIR)/man3/encrypt.3.d/encrypt.c.lint-c.cppcheck.touch \
	$(_MANDIR)/man3/envz_add.3.d/envz_add.c.lint-c.cppcheck.touch \
	$(_MANDIR)/man3/getaddrinfo_a.3.d/async.c.lint-c.cppcheck.touch \
	$(_MANDIR)/man3/getdate.3.d/getdate.c.lint-c.cppcheck.touch \
	$(_MANDIR)/man3/getgrouplist.3.d/getgrouplist.c.lint-c.cppcheck.touch \
	$(_MANDIR)/man3/getsubopt.3.d/getsubopt.c.lint-c.cppcheck.touch \
	$(_MANDIR)/man3/hsearch.3.d/hsearch.c.lint-c.cppcheck.touch \
	$(_MANDIR)/man3/malloc_info.3.d/malloc_info.c.lint-c.cppcheck.touch \
	$(_MANDIR)/man3/mallopt.3.d/mallopt.c.lint-c.cppcheck.touch \
	$(_MANDIR)/man3/matherr.3.d/matherr.c.lint-c.cppcheck.touch \
	$(_MANDIR)/man3/mcheck.3.d/mcheck.c.lint-c.cppcheck.touch \
	$(_MANDIR)/man3/mtrace.3.d/t_mtrace.c.lint-c.cppcheck.touch \
	$(_MANDIR)/man3/newlocale.3.d/newlocale.c.lint-c.cppcheck.touch \
	$(_MANDIR)/man3/pthread_attr_init.3.d/pthread_attr_init.c.lint-c.cppcheck.touch \
	$(_MANDIR)/man3/pthread_create.3.d/pthread_create.c.lint-c.cppcheck.touch \
	$(_MANDIR)/man3/pthread_getattr_np.3.d/pthread_getattr_np.c.lint-c.cppcheck.touch \
	$(_MANDIR)/man3/pthread_getcpuclockid.3.d/pthread_getcpuclockid.c.lint-c.cppcheck.touch \
	$(_MANDIR)/man3/pthread_setschedparam.3.d/pthreads_sched_test.c.lint-c.cppcheck.touch \
	$(_MANDIR)/man3/setaliasent.3.d/setaliasent.c.lint-c.cppcheck.touch \
	$(_MANDIR)/man3/setbuf.3.d/setbuf.c.lint-c.cppcheck.touch \
	$(_MANDIR)/man3/shm_open.3.d/pshm_ucase_send.c.lint-c.cppcheck.touch \
	$(_MANDIR)/man3/stpncpy.3.d/stpncpy.c.lint-c.cppcheck.touch \
	$(_MANDIR)/man3/strftime.3.d/strftime.c.lint-c.cppcheck.touch \
	$(_MANDIR)/man3/strncat.3.d/strncat.c.lint-c.cppcheck.touch \
	$(_MANDIR)/man3/strsep.3.d/strsep.c.lint-c.cppcheck.touch \
	$(_MANDIR)/man3/strtok.3.d/strtok.c.lint-c.cppcheck.touch \
	$(_MANDIR)/man3/strtol.3.d/strtol.c.lint-c.cppcheck.touch \
	$(_MANDIR)/man3/tsearch.3.d/tsearch.c.lint-c.cppcheck.touch


_LINT_c_EX_cppcheck   := $(patsubst %, %.lint-c.cppcheck.touch, $(_EX_TU_src))
ifeq ($(SKIP_XFAIL),yes)
_LINT_c_EX_cppcheck   := $(filter-out $(_XFAIL_LINT_c_EX_cppcheck), $(_LINT_c_EX_cppcheck))
endif
_LINT_c_cppcheck      := $(_LINT_c_EX_cppcheck)


$(_LINT_c_EX_cppcheck): %.lint-c.cppcheck.touch: %
$(_LINT_c_cppcheck): $(CPPCHECK_SUPPRESS) $(MK) | $$(@D)/


$(_LINT_c_EX_cppcheck):
	$(info	$(INFO_)CPPCHECK	$@)
	$(CPPCHECK) $(CPPCHECKFLAGS_) $<
	$(TOUCH) $@


.PHONY: lint-c-cppcheck
lint-c-cppcheck: $(_LINT_c_cppcheck);


endif  # include guard
