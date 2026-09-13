# Copyright 2021-2024, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_BUILD_EX_CC_INCLUDED
MAKEFILE_BUILD_EX_CC_INCLUDED := 1


include $(MAKEFILEDIR)/build/_.mk
include $(MAKEFILEDIR)/build/examples/src.mk
include $(MAKEFILEDIR)/configure/build-depends/gcc/cc.mk
include $(MAKEFILEDIR)/configure/build-depends/cpp/cpp.mk
include $(MAKEFILEDIR)/configure/xfail.mk


_XFAIL_EX_TU_o := \
	$(_MANDIR)/man2/bpf.2.d/bpf.o \
	$(_MANDIR)/man2/seccomp.2.d/seccomp.o \
	$(_MANDIR)/man2/seccomp_unotify.2.d/seccomp_unotify.o \
	$(_MANDIR)/man2/sigaction.2.d/sigaction.o \
	$(_MANDIR)/man2/spu_run.2.d/spu_run.o \
	$(_MANDIR)/man2/_syscall.2.d/_syscall.o \
	$(_MANDIR)/man2const/PR_RISCV_SET_ICACHE_FLUSH_CTX.2const.d/cmodx.o \
	$(_MANDIR)/man3/circleq.3.d/circleq.o \
	$(_MANDIR)/man3/encrypt.3.d/encrypt.o \
	$(_MANDIR)/man3/getsubopt.3.d/getsubopt.o \
	$(_MANDIR)/man3/hsearch.3.d/hsearch.o \
	$(_MANDIR)/man3/malloc_info.3.d/malloc_info.o \
	$(_MANDIR)/man3/mallopt.3.d/mallopt.o \
	$(_MANDIR)/man3/matherr.3.d/matherr.o \
	$(_MANDIR)/man3/mcheck.3.d/mcheck.o \
	$(_MANDIR)/man3/mtrace.3.d/t_mtrace.o \
	$(_MANDIR)/man3/__ppc_get_timebase.3.d/__ppc_get_timebase.o \
	$(_MANDIR)/man3/pthread_getcpuclockid.3.d/pthread_getcpuclockid.o \
	$(_MANDIR)/man3/rtime.3.d/rtime.o \
	$(_MANDIR)/man3/sem_wait.3.d/sem_wait.o \
	$(_MANDIR)/man3/setbuf.3.d/setbuf.o \
	$(_MANDIR)/man3/stpncpy.3.d/stpncpy.o \
	$(_MANDIR)/man3head/printf.h.3head.d/register_printf_specifier.o


_EX_TU_o := $(patsubst %.c, %.o, $(_EX_TU_c))
ifeq ($(SKIP_XFAIL),yes)
_EX_TU_o := $(filter-out $(_XFAIL_EX_TU_o), $(_EX_TU_o))
endif


$(_EX_TU_o): %.o: %.c $(MK)
	$(info	$(INFO_)CC		$@)
	$(CC) -c $(CFLAGS_) $(CPPFLAGS_) -o $@ $<


.PHONY: build-ex-cc
build-ex-cc:  $(_EX_TU_o);


endif  # include guard
