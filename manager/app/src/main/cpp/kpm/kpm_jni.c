/* SPDX-License-Identifier: GPL-2.0-or-later */
/*
 * JNI 包装：让管理器在自己的进程里直接调用 KernelPatch 的 supercall，
 * 不再 fork kpatch / 走 root shell。
 *
 * 依赖 KernelPatch 的 supercall.h / kpm.c（KernelSU-Next/KPatch-Next, 0.13.5）。
 */

#include <jni.h>
#include <fcntl.h>
#include <stdio.h>
#include <unistd.h>

#include "kpm.h"

/* kpm.h 只声明了 load/unload/nums/list/info，control 需要自行声明 */
int kpm_control(const char *name, const char *ctl_args);

static const char *g_info_name = NULL;

static int info_trampoline(void) {
    return kpm_info(g_info_name);
}

/* 把 stdout 重定向到文件后执行 fn，Kotlin 侧读文件拿输出 */
static int run_capture(const char *out_path, int (*fn)(void)) {
    if (out_path == NULL) return -2;
    fflush(stdout);
    int saved = dup(STDOUT_FILENO);
    if (saved < 0) return -3;
    int fd = open(out_path, O_WRONLY | O_CREAT | O_TRUNC, 0600);
    if (fd < 0) {
        close(saved);
        return -4;
    }
    dup2(fd, STDOUT_FILENO);
    int rc = fn();
    fflush(stdout);
    dup2(saved, STDOUT_FILENO);
    close(saved);
    close(fd);
    return rc;
}

static const char *to_utf(JNIEnv *env, jstring s) {
    return s == NULL ? NULL : (*env)->GetStringUTFChars(env, s, NULL);
}

static void free_utf(JNIEnv *env, jstring s, const char *p) {
    if (s != NULL && p != NULL) (*env)->ReleaseStringUTFChars(env, s, p);
}

JNIEXPORT jint JNICALL Java_org_bakasu_bakasu_ui_screen_kpm_KpmNatives_load
(JNIEnv *env, jobject thiz, jstring path, jstring args) {
    const char *p = to_utf(env, path);
    const char *a = to_utf(env, args);
    int rc = kpm_load(p, a);
    free_utf(env, path, p);
    free_utf(env, args, a);
    return rc;
}

JNIEXPORT jint JNICALL Java_org_bakasu_bakasu_ui_screen_kpm_KpmNatives_unload
(JNIEnv *env, jobject thiz, jstring name) {
    const char *n = to_utf(env, name);
    int rc = kpm_unload(n);
    free_utf(env, name, n);
    return rc;
}

JNIEXPORT jint JNICALL Java_org_bakasu_bakasu_ui_screen_kpm_KpmNatives_nums
(JNIEnv *env, jobject thiz) {
    return kpm_nums();
}

JNIEXPORT jint JNICALL Java_org_bakasu_bakasu_ui_screen_kpm_KpmNatives_control
(JNIEnv *env, jobject thiz, jstring name, jstring args) {
    const char *n = to_utf(env, name);
    const char *a = to_utf(env, args);
    int rc = kpm_control(n, a);
    free_utf(env, name, n);
    free_utf(env, args, a);
    return rc;
}

JNIEXPORT jint JNICALL Java_org_bakasu_bakasu_ui_screen_kpm_KpmNatives_list
(JNIEnv *env, jobject thiz, jstring outPath) {
    const char *o = to_utf(env, outPath);
    int rc = run_capture(o, kpm_list);
    free_utf(env, outPath, o);
    return rc;
}

JNIEXPORT jint JNICALL Java_org_bakasu_bakasu_ui_screen_kpm_KpmNatives_info
(JNIEnv *env, jobject thiz, jstring name, jstring outPath) {
    const char *n = to_utf(env, name);
    const char *o = to_utf(env, outPath);
    g_info_name = n;
    int rc = run_capture(o, info_trampoline);
    g_info_name = NULL;
    free_utf(env, name, n);
    free_utf(env, outPath, o);
    return rc;
}
