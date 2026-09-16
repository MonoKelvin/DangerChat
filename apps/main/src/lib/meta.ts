import pkg from '../../../../package.json';
import tauriConf from '../../../../src-tauri/tauri.conf.json';

/** 应用元数据唯一引用口：名称取自 src-tauri/tauri.conf.json 的 productName；
 *  版本 / 作者 / 仓库 / 协议取自根 package.json（tauri.conf.json 的 version 同源指向它）。
 *  组件里不要再写这些字面量。 */
export const APP_NAME = tauriConf.productName;
export const APP_VERSION = pkg.version;
export const AUTHOR = pkg.author;
export const AUTHOR_URL = `https://github.com/${pkg.author}`;
export const LICENSE = pkg.license;
export const REPO_URL = pkg.repository.url.replace(/^git\+/, '').replace(/\.git$/, '');
