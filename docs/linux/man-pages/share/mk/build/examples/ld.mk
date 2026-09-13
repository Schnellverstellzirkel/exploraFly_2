# Copyright 2021-2024, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_BUILD_EX_LD_INCLUDED
MAKEFILE_BUILD_EX_LD_INCLUDED := 1


include $(MAKEFILEDIR)/build/_.mk
include $(MAKEFILEDIR)/build/examples/cc.mk
include $(MAKEFILEDIR)/configure/build-depends/binutils/ld.mk
include $(MAKEFILEDIR)/configure/xfail.mk


_XFAIL_EX_TU_bin := \
	$(_MANDIR)/man2/add_key.2.d/add_key \
	$(_MANDIR)/man2/keyctl.2.d/key_instantiate \
	$(_MANDIR)/man2/request_key.2.d/t_request_key \
	$(_MANDIR)/man2/select_tut.2.d/select \
	$(_MANDIR)/man2/shmop.2.d/svshm_string_read \
	$(_MANDIR)/man3/cacos.3.d/cacos \
	$(_MANDIR)/man3/cacosh.3.d/cacosh \
	$(_MANDIR)/man3/catan.3.d/catan \
	$(_MANDIR)/man3/catanh.3.d/catanh \
	$(_MANDIR)/man3/getaddrinfo_a.3.d/async \
	$(_MANDIR)/man3/inet_net_pton.3.d/inet_net_pton \
	$(_MANDIR)/man3/list.3.d/list \
	$(_MANDIR)/man3/mallinfo.3.d/mallinfo \
	$(_MANDIR)/man3/slist.3.d/slist \
	$(_MANDIR)/man3/stailq.3.d/stailq \
	$(_MANDIR)/man3/strncat.3.d/strncat \
	$(_MANDIR)/man3/tailq.3.d/tailq \
	$(_MANDIR)/man3/tsearch.3.d/tsearch


_EX_TU_bin := $(patsubst %.o, %, $(_EX_TU_o))
ifeq ($(SKIP_XFAIL),yes)
_EX_TU_bin := $(filter-out $(_XFAIL_EX_TU_bin), $(_EX_TU_bin))
endif


$(_EX_TU_bin): %: %.o $(MK)
	$(info	$(INFO_)LD		$@)
	$(LD) $(LDFLAGS_) -o $@ $< $(LDLIBS_)


.PHONY: build-ex-ld
build-ex-ld: $(_EX_TU_bin);


endif  # include guard
