# Copyright 2021-2024, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_LINT_C_IWYU_INCLUDED
MAKEFILE_LINT_C_IWYU_INCLUDED := 1


include $(MAKEFILEDIR)/build/_.mk
include $(MAKEFILEDIR)/build/examples/src.mk
include $(MAKEFILEDIR)/configure/build-depends/clang/clang.mk
include $(MAKEFILEDIR)/configure/build-depends/coreutils/tac.mk
include $(MAKEFILEDIR)/configure/build-depends/coreutils/touch.mk
include $(MAKEFILEDIR)/configure/build-depends/coreutils/true.mk
include $(MAKEFILEDIR)/configure/build-depends/cpp/cpp.mk
include $(MAKEFILEDIR)/configure/build-depends/grep/grep.mk
include $(MAKEFILEDIR)/configure/build-depends/iwyu/iwyu.mk
include $(MAKEFILEDIR)/configure/build-depends/sed/sed.mk
include $(MAKEFILEDIR)/configure/xfail.mk


_XFAIL_LINT_c_EX_iwyu := \
	$(_MANDIR)/man2/clock_getres.2.d/clock_getres.c.lint-c.iwyu.touch \
	$(_MANDIR)/man2/getrlimit.2.d/getrlimit.c.lint-c.iwyu.touch \
	$(_MANDIR)/man2/listxattr.2.d/listxattr.c.lint-c.iwyu.touch \
	$(_MANDIR)/man2/mount_setattr.2.d/mount_setattr.c.lint-c.iwyu.touch \
	$(_MANDIR)/man2/recvmmsg.2.d/recvmmsg.c.lint-c.iwyu.touch \
	$(_MANDIR)/man2/seccomp.2.d/seccomp.c.lint-c.iwyu.touch \
	$(_MANDIR)/man2/seccomp_unotify.2.d/seccomp_unotify.c.lint-c.iwyu.touch \
	$(_MANDIR)/man2/select.2.d/select.c.lint-c.iwyu.touch \
	$(_MANDIR)/man2/semget.2.d/t_semget.c.lint-c.iwyu.touch \
	$(_MANDIR)/man2/sendmmsg.2.d/sendmmsg.c.lint-c.iwyu.touch \
	$(_MANDIR)/man2/shmop.2.d/svshm_string.h.lint-c.iwyu.touch \
	$(_MANDIR)/man2/_syscall.2.d/_syscall.c.lint-c.iwyu.touch \
	$(_MANDIR)/man2/timer_create.2.d/timer_create.c.lint-c.iwyu.touch \
	$(_MANDIR)/man2/userfaultfd.2.d/userfaultfd.c.lint-c.iwyu.touch \
	$(_MANDIR)/man2const/TIOCMSET.2const.d/tiocmget.c.lint-c.iwyu.touch \
	$(_MANDIR)/man3/backtrace.3.d/backtrace.c.lint-c.iwyu.touch \
	$(_MANDIR)/man3/bsearch.3.d/bsearch.c.lint-c.iwyu.touch \
	$(_MANDIR)/man3/bswap.3.d/bswap.c.lint-c.iwyu.touch \
	$(_MANDIR)/man3/cacos.3.d/cacos.c.lint-c.iwyu.touch \
	$(_MANDIR)/man3/cacosh.3.d/cacosh.c.lint-c.iwyu.touch \
	$(_MANDIR)/man3/catan.3.d/catan.c.lint-c.iwyu.touch \
	$(_MANDIR)/man3/catanh.3.d/catanh.c.lint-c.iwyu.touch \
	$(_MANDIR)/man3/clock_getcpuclockid.3.d/clock_getcpuclockid.c.lint-c.iwyu.touch \
	$(_MANDIR)/man3/CPU_SET.3.d/CPU_SET.c.lint-c.iwyu.touch \
	$(_MANDIR)/man3/dl_iterate_phdr.3.d/dl_iterate_phdr.c.lint-c.iwyu.touch \
	$(_MANDIR)/man3/dlinfo.3.d/dlinfo.c.lint-c.iwyu.touch \
	$(_MANDIR)/man3/duplocale.3.d/duplocale.c.lint-c.iwyu.touch \
	$(_MANDIR)/man3/endian.3.d/endian.c.lint-c.iwyu.touch \
	$(_MANDIR)/man3/envz_add.3.d/envz_add.c.lint-c.iwyu.touch \
	$(_MANDIR)/man3/fopencookie.3.d/fopencookie.c.lint-c.iwyu.touch \
	$(_MANDIR)/man3/frexp.3.d/frexp.c.lint-c.iwyu.touch \
	$(_MANDIR)/man3/ftw.3.d/ftw.c.lint-c.iwyu.touch \
	$(_MANDIR)/man3/_Generic.3.d/_Generic.c.lint-c.iwyu.touch \
	$(_MANDIR)/man3/getaddrinfo.3.d/client.c.lint-c.iwyu.touch \
	$(_MANDIR)/man3/getaddrinfo.3.d/server.c.lint-c.iwyu.touch \
	$(_MANDIR)/man3/getgrouplist.3.d/getgrouplist.c.lint-c.iwyu.touch \
	$(_MANDIR)/man3/getline.3.d/getline.c.lint-c.iwyu.touch \
	$(_MANDIR)/man3/getopt.3.d/getopt_long.c.lint-c.iwyu.touch \
	$(_MANDIR)/man3/getprotoent_r.3.d/getprotoent_r.c.lint-c.iwyu.touch \
	$(_MANDIR)/man3/getservent_r.3.d/getservent_r.c.lint-c.iwyu.touch \
	$(_MANDIR)/man3/getsubopt.3.d/getsubopt.c.lint-c.iwyu.touch \
	$(_MANDIR)/man3/if_nameindex.3.d/if_nameindex.c.lint-c.iwyu.touch \
	$(_MANDIR)/man3/inet.3.d/inet.c.lint-c.iwyu.touch \
	$(_MANDIR)/man3/inet_net_pton.3.d/inet_net_pton.c.lint-c.iwyu.touch \
	$(_MANDIR)/man3/inet_pton.3.d/inet_pton.c.lint-c.iwyu.touch \
	$(_MANDIR)/man3/list.3.d/list.c.lint-c.iwyu.touch \
	$(_MANDIR)/man3/mallinfo.3.d/mallinfo.c.lint-c.iwyu.touch \
	$(_MANDIR)/man3/malloc_info.3.d/malloc_info.c.lint-c.iwyu.touch \
	$(_MANDIR)/man3/mbstowcs.3.d/mbstowcs.c.lint-c.iwyu.touch \
	$(_MANDIR)/man3/mq_getattr.3.d/mq_getattr.c.lint-c.iwyu.touch \
	$(_MANDIR)/man3/mq_notify.3.d/mq_notify.c.lint-c.iwyu.touch \
	$(_MANDIR)/man3/mtrace.3.d/t_mtrace.c.lint-c.iwyu.touch \
	$(_MANDIR)/man3/newlocale.3.d/newlocale.c.lint-c.iwyu.touch \
	$(_MANDIR)/man3/posix_spawn.3.d/posix_spawn.c.lint-c.iwyu.touch \
	$(_MANDIR)/man3/pthread_attr_init.3.d/pthread_attr_init.c.lint-c.iwyu.touch \
	$(_MANDIR)/man3/pthread_cleanup_push.3.d/pthread_cleanup_push.c.lint-c.iwyu.touch \
	$(_MANDIR)/man3/pthread_getattr_default_np.3.d/pthread_getattr_default_np.c.lint-c.iwyu.touch \
	$(_MANDIR)/man3/pthread_getattr_np.3.d/pthread_getattr_np.c.lint-c.iwyu.touch \
	$(_MANDIR)/man3/pthread_getcpuclockid.3.d/pthread_getcpuclockid.c.lint-c.iwyu.touch \
	$(_MANDIR)/man3/pthread_setaffinity_np.3.d/pthread_setaffinity_np.c.lint-c.iwyu.touch \
	$(_MANDIR)/man3/pthread_setname_np.3.d/pthread_setname_np.c.lint-c.iwyu.touch \
	$(_MANDIR)/man3/pthread_setschedparam.3.d/pthreads_sched_test.c.lint-c.iwyu.touch \
	$(_MANDIR)/man3/pthread_sigmask.3.d/pthread_sigmask.c.lint-c.iwyu.touch \
	$(_MANDIR)/man3/sem_wait.3.d/sem_wait.c.lint-c.iwyu.touch \
	$(_MANDIR)/man3/shm_open.3.d/pshm_ucase.h.lint-c.iwyu.touch \
	$(_MANDIR)/man3/shm_open.3.d/pshm_ucase_bounce.c.lint-c.iwyu.touch \
	$(_MANDIR)/man3/shm_open.3.d/pshm_ucase_send.c.lint-c.iwyu.touch \
	$(_MANDIR)/man3/slist.3.d/slist.c.lint-c.iwyu.touch \
	$(_MANDIR)/man3/stailq.3.d/stailq.c.lint-c.iwyu.touch \
	$(_MANDIR)/man3/tailq.3.d/tailq.c.lint-c.iwyu.touch


_LINT_c_EX_iwyu   := $(patsubst %, %.lint-c.iwyu.touch, $(_EX_TU_src))
ifeq ($(SKIP_XFAIL),yes)
_LINT_c_EX_iwyu   := $(filter-out $(_XFAIL_LINT_c_EX_iwyu), $(_LINT_c_EX_iwyu))
endif
_LINT_c_iwyu      := $(_LINT_c_EX_iwyu)


$(_LINT_c_EX_iwyu): %.lint-c.iwyu.touch: %
$(_LINT_c_iwyu): $(MK) | $$(@D)/


$(_LINT_c_iwyu):
	$(info	$(INFO_)IWYU		$@)
	! ($(IWYU) $(IWYUFLAGS_) $(CLANGFLAGS_) $(CPPFLAGS_) $< 2>&1 \
	   | $(SED) -n '/should add these lines:/,$$p' \
	   | $(TAC) \
	   | $(SED) '/correct/{N;d}' \
	   | $(TAC) \
	   || $(TRUE); \
	) \
	| $(GREP) ^ >&2
	$(TOUCH) $@


.PHONY: lint-c-iwyu
lint-c-iwyu: $(_LINT_c_iwyu);


endif  # include guard
