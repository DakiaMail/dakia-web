import {afterEach,beforeEach,expect,it,vi} from 'vitest';
import {act,cleanup,fireEvent,render,screen} from '@testing-library/react';
import {CopyButton,DakiaProvider} from '@dakia/ui';
beforeEach(()=>vi.stubGlobal('matchMedia',vi.fn(()=>({matches:false,addEventListener:vi.fn(),removeEventListener:vi.fn()}))));
afterEach(()=>{cleanup();vi.useRealTimers();vi.unstubAllGlobals();});
it('confirms only after a successful write, keeps focus, and resets without shifting the value',async()=>{
 vi.useFakeTimers();let finish!:()=>void;const writeText=vi.fn(()=>new Promise<void>(r=>finish=r));vi.stubGlobal('navigator',{clipboard:{writeText}});
 render(<DakiaProvider><CopyButton value="ABC123">Reference: ABC123</CopyButton></DakiaProvider>);
 const button=screen.getByRole('button',{name:'Reference: ABC123'});button.focus();fireEvent.click(button);fireEvent.click(button);
 expect(writeText).toHaveBeenCalledExactlyOnceWith('ABC123');expect(button.getAttribute('data-copy-state')).toBe('pending');expect(screen.getByRole('status').textContent).toBe('');
 expect(document.activeElement).toBe(button);await act(async()=>finish());
 expect(button.getAttribute('data-copy-state')).toBe('copied');expect(screen.getByRole('status').textContent).toBe('Copied');expect(button.textContent).toBe('Reference: ABC123');
 act(()=>vi.advanceTimersByTime(1800));expect(button.getAttribute('data-copy-state')).toBe('idle');expect(document.activeElement).toBe(button);
});
it('announces a denied copy and allows retry without a false success',async()=>{
 const writeText=vi.fn().mockRejectedValueOnce(new Error('denied')).mockResolvedValue(undefined);vi.stubGlobal('navigator',{clipboard:{writeText}});
 render(<DakiaProvider><CopyButton value="123456"/></DakiaProvider>);const button=screen.getByRole('button',{name:'123456'});
 fireEvent.click(button);await screen.findByRole('alert');expect(button.getAttribute('data-copy-state')).toBe('error');expect(button.title).toContain('Couldn’t copy');
 fireEvent.click(button);await screen.findByText('Copied');expect(writeText).toHaveBeenCalledTimes(2);expect(screen.queryByRole('alert')).toBeNull();
});
it('does not attach a late copy failure to a changed value',async()=>{
 let fail!:(e:Error)=>void;const writeText=vi.fn().mockImplementationOnce(()=>new Promise<void>((_,r)=>fail=r)).mockResolvedValue(undefined);vi.stubGlobal('navigator',{clipboard:{writeText}});
 const show=(value:string)=><DakiaProvider><CopyButton value={value}/></DakiaProvider>;const view=render(show('old'));fireEvent.click(screen.getByRole('button'));
 view.rerender(show('new'));fireEvent.click(screen.getByRole('button'));await screen.findByText('Copied');await act(async()=>fail(new Error('old failed')));
 expect(screen.getByRole('button',{name:'new'}).getAttribute('data-copy-state')).toBe('copied');expect(screen.queryByRole('alert')).toBeNull();
});
it('handles unavailable clipboard access and never copies a disabled control',async()=>{
 vi.stubGlobal('navigator',{});const view=render(<DakiaProvider><CopyButton value="123456" disabled/></DakiaProvider>);
 fireEvent.click(screen.getByRole('button'));expect(screen.queryByRole('alert')).toBeNull();
 view.rerender(<DakiaProvider><CopyButton value="123456"/></DakiaProvider>);fireEvent.click(screen.getByRole('button'));await screen.findByRole('alert');
});
