#!/system/bin/sh
# BSR per-instance Magisk gate. Activates Magisk ONLY if THIS instance (its own /data) is flagged.
[ -f /data/adb/.bsr_root ] || exit 0
M=/system/etc/init/magisk
case "$1" in
  post-fs-data)
    "$M/magiskpolicy" --live --magisk 2>/dev/null
    "$M/magisk64" --auto-selinux --setup-sbin "$M" /sbin 2>/dev/null
    /sbin/magisk --auto-selinux --post-fs-data 2>/dev/null
    ;;
  service)        /sbin/magisk --auto-selinux --service 2>/dev/null ;;
  boot-complete)  mkdir -p /data/adb/magisk; /sbin/magisk --auto-selinux --boot-complete 2>/dev/null ;;
  zygote-restart) /sbin/magisk --auto-selinux --zygote-restart 2>/dev/null ;;
esac
exit 0