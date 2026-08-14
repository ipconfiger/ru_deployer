-- api-manager-platform 独立管理库（mysql 容器首次启动时自动执行）
-- sh_admin 用户由 MYSQL_USER/MYSQL_PASSWORD 创建，仅默认授权 sh_admin 库；
-- 这里补齐 platform_admin 库及授权。
CREATE DATABASE IF NOT EXISTS platform_admin CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;
GRANT ALL PRIVILEGES ON platform_admin.* TO 'sh_admin'@'%';
FLUSH PRIVILEGES;
