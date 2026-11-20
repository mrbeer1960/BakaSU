/* SPDX-License-Identifier: GPL-2.0 */
#ifndef __KSU_H_KPM
#define __KSU_H_KPM

/*
 * Built-in KPM (Kernel Patch Module) backend.
 *
 * Step 1: only the feature probe (KSU_FEATURE_KPM) exists. When the backend is
 *         compiled in (CONFIG_KSU_KPM_ENABLED=y) the feature is registered and
 *         reports availability; the manager shows the KPM entry only when the
 *         kernel reports the feature as supported.
 * Step 2: the real loader (KernelPatch bridge / ELF module loader) plugs into
 *         kpm_backend_available() and kpm_ioctl().
 */

void ksu_kpm_init(void);
void ksu_kpm_exit(void);

/* Whether a usable KPM loader is present in this kernel build. */
bool kpm_backend_available(void);

#endif /* __KSU_H_KPM */
