# Copyright 2021-2024, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_LINT_C_CLANG_TIDY_INCLUDED
MAKEFILE_LINT_C_CLANG_TIDY_INCLUDED := 1


include $(MAKEFILEDIR)/build/_.mk
include $(MAKEFILEDIR)/build/examples/src.mk
include $(MAKEFILEDIR)/configure/build-depends/clang/clang.mk
include $(MAKEFILEDIR)/configure/build-depends/clang-tidy/clang-tidy.mk
include $(MAKEFILEDIR)/configure/build-depends/coreutils/touch.mk
include $(MAKEFILEDIR)/configure/build-depends/cpp/cpp.mk
include $(MAKEFILEDIR)/configure/build-depends/sed/sed.mk
include $(MAKEFILEDIR)/configure/xfail.mk


_XFAIL_LINT_c_EX_clang_tidy := \
	$(_MANDIR)/man2/add_key.2.d/add_key.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/bpf.2.d/bpf.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/chown.2.d/chown.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/clone.2.d/clone.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/close_range.2.d/close_range.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/copy_file_range.2.d/copy_file_range.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/eventfd.2.d/eventfd.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/execve.2.d/execve.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/execve.2.d/myecho.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/futex.2.d/futex.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/getdents.2.d/getdents.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/getrlimit.2.d/getrlimit.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/ioctl_fat.2.d/display_fat_volume_id.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/ioctl_fat.2.d/ioctl_fat.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/ioctl_fat.2.d/toggle_fat_archive_flag.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/ioctl_ns.2.d/ns_show.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/ioctl_tty.2.d/tcgets.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/kcmp.2.d/kcmp.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/keyctl.2.d/key_instantiate.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/listxattr.2.d/listxattr.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/membarrier.2.d/membarrier.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/memfd_create.2.d/t_get_seals.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/memfd_create.2.d/t_memfd_create.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/mmap.2.d/mmap.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/mount_setattr.2.d/mount_setattr.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/mprotect.2.d/mprotect.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/msgop.2.d/msgop.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/open_by_handle_at.2.d/t_name_to_handle_at.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/open_by_handle_at.2.d/t_open_by_handle_at.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/perf_event_open.2.d/perf_event_open.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/pidfd_open.2.d/pidfd_open.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/pidfd_send_signal.2.d/pidfd_send_signal.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/pipe.2.d/pipe.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/pivot_root.2.d/pivot_root.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/poll.2.d/poll_input.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/process_vm_readv.2.d/process_vm_readv.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/readlink.2.d/readlink.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/recvmmsg.2.d/recvmmsg.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/request_key.2.d/t_request_key.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/sched_setaffinity.2.d/sched_setaffinity.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/seccomp.2.d/seccomp.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/seccomp_unotify.2.d/seccomp_unotify.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/select_tut.2.d/select.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/semget.2.d/t_semget.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/sendmmsg.2.d/sendmmsg.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/setns.2.d/setns.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/shmop.2.d/svshm_string_read.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/shmop.2.d/svshm_string_write.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/sigaction.2.d/sigaction.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/spu_run.2.d/spu_run.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/stat.2.d/stat.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/_syscall.2.d/_syscall.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/syscall.2.d/syscall.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/sysctl.2.d/sysctl.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/tee.2.d/tee.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/timer_create.2.d/timer_create.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/timerfd_create.2.d/timerfd_create.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/unshare.2.d/unshare.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/userfaultfd.2.d/userfaultfd.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2/wait.2.d/wait.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2const/PR_RISCV_SET_ICACHE_FLUSH_CTX.2const.d/cmodx.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man2const/TCSETS.2const.d/tcgets.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/atexit.3.d/atexit.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/backtrace.3.d/backtrace.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/bsearch.3.d/bsearch.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/bswap.3.d/bswap.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/cacos.3.d/cacos.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/cacosh.3.d/cacosh.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/catan.3.d/catan.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/catanh.3.d/catanh.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/circleq.3.d/circleq.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/clock_getcpuclockid.3.d/clock_getcpuclockid.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/CPU_SET.3.d/CPU_SET.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/dl_iterate_phdr.3.d/dl_iterate_phdr.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/dlinfo.3.d/dlinfo.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/duplocale.3.d/duplocale.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/encrypt.3.d/encrypt.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/end.3.d/end.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/endian.3.d/endian.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/envz_add.3.d/envz_add.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/fmemopen.3.d/fmemopen.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/fopencookie.3.d/fopencookie.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/fread.3.d/fread.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/frexp.3.d/frexp.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/ftw.3.d/ftw.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/_Generic.3.d/_Generic.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/getaddrinfo.3.d/client.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/getaddrinfo.3.d/server.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/getaddrinfo_a.3.d/async.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/getaddrinfo_a.3.d/sync.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/getdate.3.d/getdate.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/getgrent_r.3.d/getgrent_r.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/getgrouplist.3.d/getgrouplist.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/getline.3.d/getline.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/getopt.3.d/getopt.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/getopt.3.d/getopt_long.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/getprotoent_r.3.d/getprotoent_r.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/getpwnam.3.d/getpwnam.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/getservent_r.3.d/getservent_r.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/getsubopt.3.d/getsubopt.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/getutent.3.d/getutent.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/hsearch.3.d/hsearch.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/if_nameindex.3.d/if_nameindex.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/inet.3.d/inet.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/inet_net_pton.3.d/inet_net_pton.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/inet_pton.3.d/inet_pton.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/insque.3.d/insque.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/list.3.d/list.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/mallinfo.3.d/mallinfo.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/malloc_info.3.d/malloc_info.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/mallopt.3.d/mallopt.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/matherr.3.d/matherr.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/MAX.3.d/MAX.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/mbstowcs.3.d/mbstowcs.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/mq_getattr.3.d/mq_getattr.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/mq_notify.3.d/mq_notify.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/mtrace.3.d/t_mtrace.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/newlocale.3.d/newlocale.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/offsetof.3.d/offsetof.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/posix_spawn.3.d/posix_spawn.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/__ppc_get_timebase.3.d/__ppc_get_timebase.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/pthread_attr_init.3.d/pthread_attr_init.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/pthread_cleanup_push.3.d/pthread_cleanup_push.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/pthread_create.3.d/pthread_create.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/pthread_getattr_np.3.d/pthread_getattr_np.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/pthread_setname_np.3.d/pthread_setname_np.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/pthread_setschedparam.3.d/pthreads_sched_test.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/qsort.3.d/qsort.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/rand.3.d/rand.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/rpmatch.3.d/rpmatch.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/rtime.3.d/rtime.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/scandir.3.d/scandir.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/sem_wait.3.d/sem_wait.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/setbuf.3.d/setbuf.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/shm_open.3.d/pshm_ucase_bounce.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/shm_open.3.d/pshm_ucase_send.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/slist.3.d/slist.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/stailq.3.d/stailq.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/static_assert.3.d/must_be.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/stpncpy.3.d/stpncpy.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/strcmp.3.d/string_comp.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/strcpy.3.d/strcpy.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/strftime.3.d/strftime.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/strsep.3.d/strsep.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/strtok.3.d/strtok.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/strtol.3.d/strtol.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/strverscmp.3.d/strverscmp.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/tailq.3.d/tailq.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/tsearch.3.d/tsearch.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3/wordexp.3.d/wordexp.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3const/EXIT_SUCCESS.3const.d/EXIT_SUCCESS.c.lint-c.clang-tidy.touch \
	$(_MANDIR)/man3head/printf.h.3head.d/register_printf_specifier.c.lint-c.clang-tidy.touch


_LINT_c_EX_clang_tidy   := $(patsubst %, %.lint-c.clang-tidy.touch, $(_EX_TU_src))
ifeq ($(SKIP_XFAIL),yes)
_LINT_c_EX_clang_tidy   := $(filter-out $(_XFAIL_LINT_c_EX_clang_tidy), $(_LINT_c_EX_clang_tidy))
endif
_LINT_c_clang_tidy      := $(_LINT_c_EX_clang_tidy)


$(_LINT_c_EX_clang_tidy): %.lint-c.clang-tidy.touch: %
$(_LINT_c_clang_tidy): $(CLANG_TIDY_CONF) $(MK) | $$(@D)/


$(_LINT_c_clang_tidy):
	$(info	$(INFO_)CLANG_TIDY	$@)
	$(CLANG_TIDY) $(CLANG_TIDYFLAGS_) $< -- $(CLANGFLAGS_) $(CPPFLAGS_) 2>&1 \
	| $(SED) '/generated\.$$/d' >&2
	$(TOUCH) $@


.PHONY: lint-c-clang-tidy
lint-c-clang-tidy: $(_LINT_c_clang_tidy);


endif  # include guard
