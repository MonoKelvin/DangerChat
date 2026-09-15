import { useEffect } from 'react';

/**
 * 自定义鼠标指针（参考 monokelvin.studio cursorfx）：箭头 / 交互圆环 / 文本
 * I-beam 三态。容器 transform 定位（GPU 合成、零 layout），箭头尖端 1:1 跟随
 * 鼠标坐标；开启时全局 cursor:none，滚动条拖拽期间临时放行系统指针。
 * 挂载即启用，卸载即还原；开关由设置「通用 → 外观」控制（默认开启）。
 */
export function CursorFx() {
  useEffect(() => {
    const root = document.documentElement;
    const finePointer = window.matchMedia('(pointer: fine)');
    if (!finePointer.matches || document.querySelector('.cursor-fx')) return;

    const cursor = document.createElement('div');
    cursor.className = 'cursor-fx';
    cursor.setAttribute('aria-hidden', 'true');
    cursor.innerHTML =
      '<svg class="cf-arrow" viewBox="0 0 24 24" width="26" height="26">' +
      '<path d="M5.5 3.2 L18.8 11.4 L12.6 12.8 L9.9 19.6 Z"></path></svg><i></i>';
    document.body.append(cursor);
    root.classList.add('has-cursor-fx');

    // SVG 尖端位于 viewBox (5.5, 3.2)：反向平移让尖端落在容器原点（= 鼠标坐标）
    const arrowEl = cursor.querySelector<HTMLElement>('.cf-arrow');
    if (arrowEl) arrowEl.style.transform = 'translate(-5.5px, -3.2px)';

    const linkSelector = 'button, [role=button], select, label, a, summary, [data-cursor=interactive]';
    const textSelector = 'input, textarea, [contenteditable=true]';

    let started = false;
    let textState = false;
    let linkState = false;

    const onMove = (event: MouseEvent) => {
      // transform-only 写入：不触发 layout，浏览器每帧按最后一次值合成
      cursor.style.transform = `translate3d(${event.clientX}px, ${event.clientY}px, 0)`;
      if (!started) {
        started = true;
        cursor.classList.add('on');
      }
    };

    const onOver = (event: MouseEvent) => {
      const target = event.target;
      if (!(target instanceof Element)) return;
      const nextText = Boolean(target.closest(textSelector));
      const nextLink = !nextText && Boolean(target.closest(linkSelector));
      if (nextText === textState && nextLink === linkState) return;
      textState = nextText;
      linkState = nextLink;
      cursor.classList.toggle('is-text', nextText);
      cursor.classList.toggle('is-link', nextLink);
    };

    // 滚动条拖拽：原生滚动条绘制在所有 fixed DOM 之上，期间放行系统指针
    const hitsScrollbar = (event: MouseEvent): boolean => {
      const docEl = document.documentElement;
      if (event.clientX > docEl.clientWidth || event.clientY > docEl.clientHeight) return true;
      const target = event.target;
      if (!(target instanceof HTMLElement)) return false;
      const style = getComputedStyle(target);
      const scrollY =
        (style.overflowY === 'auto' || style.overflowY === 'scroll') &&
        target.scrollHeight > target.clientHeight;
      const scrollX =
        (style.overflowX === 'auto' || style.overflowX === 'scroll') &&
        target.scrollWidth > target.clientWidth;
      if (!scrollY && !scrollX) return false;
      const rect = target.getBoundingClientRect();
      if (scrollY && event.clientX - rect.left > target.clientLeft + target.clientWidth) return true;
      if (scrollX && event.clientY - rect.top > target.clientTop + target.clientHeight) return true;
      return false;
    };

    const endScrollbar = () => {
      root.classList.remove('scrollbar-active');
      window.removeEventListener('pointerup', endScrollbar);
      window.removeEventListener('mouseup', endScrollbar);
    };
    const onDown = (event: MouseEvent) => {
      if (!hitsScrollbar(event)) return;
      root.classList.add('scrollbar-active');
      window.addEventListener('pointerup', endScrollbar);
      window.addEventListener('mouseup', endScrollbar);
    };

    window.addEventListener('mousemove', onMove, { passive: true });
    // 拖拽期间浏览器隐式捕获指针，pointermove 仍冒泡，双监听确保跟随
    window.addEventListener('pointermove', onMove, { passive: true });
    window.addEventListener('mouseover', onOver, { passive: true });
    window.addEventListener('pointerdown', onDown, { passive: true });

    return () => {
      window.removeEventListener('mousemove', onMove);
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('mouseover', onOver);
      window.removeEventListener('pointerdown', onDown);
      endScrollbar();
      root.classList.remove('has-cursor-fx');
      root.classList.remove('scrollbar-active');
      cursor.remove();
    };
  }, []);

  return null;
}
