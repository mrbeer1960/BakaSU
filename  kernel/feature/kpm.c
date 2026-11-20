// SPDX-License-Identifier: GPL-2.0
/*
 * Built-in KPM (Kernel Patch Module) support - feature registration only.
 *
 * This file is the BakaSU-side plumbing. The module loader itself (KernelPatch
 * supercall bridge) lands in step 2; until then CONFIG_KSU_KPM_ENABLED stays
 * off and the KSU_FEATURE_KPM feature is NOT registered, so the manager hides
 * the KPM entry instead of showing a dead page.
 */

#include <linux/errno.h>
#include <linux/kernel.h>
#include <linux/types.h>

#include "klog.h"
#include "policy/feature.h"
#include "uapi/feature.h"
#include "feature/kpm.h"

bool kpm_backend_available(void)
{
#ifdef CONFIG_KSU_KPM_ENABLED
	return true;
#else
	return false;
#endif
}

static int kpm_feature_get(u64 *value)
{
	if (!value)
		return -EINVAL;
	*value = kpm_backend_available() ? 1 : 0;
	return 0;
}

static int kpm_feature_set(u64 value)
{
	/* Nothing to tune yet; the backend is either compiled in or not. */
	return -EOPNOTSUPP;
}

#ifdef CONFIG_KSU_KPM_ENABLED
static const struct ksu_feature_handler kpm_feature_handler = {
	.feature_id = KSU_FEATURE_KPM,
	.name = "kpm",
	.get_handler = kpm_feature_get,
	.set_handler = kpm_feature_set,
};
#endif

void ksu_kpm_init(void)
{
#ifdef CONFIG_KSU_KPM_ENABLED
	int ret = ksu_register_feature_handler(&kpm_feature_handler);

	if (ret) {
		pr_err("kpm: failed to register feature: %d\n", ret);
		return;
	}
	pr_info("kpm: feature registered\n");
#else
	pr_info("kpm: backend not compiled in, feature hidden\n");
#endif
}

void ksu_kpm_exit(void)
{
#ifdef CONFIG_KSU_KPM_ENABLED
	ksu_unregister_feature_handler(KSU_FEATURE_KPM);
#endif
}
