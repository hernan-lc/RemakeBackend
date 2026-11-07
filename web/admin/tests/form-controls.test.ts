import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'
import AppInput from '../src/components/ui/AppInput.vue'
import AppSelect from '../src/components/ui/AppSelect.vue'

describe('shared form control accessibility', () => {
  it('disables the actual input while preserving its label and value', () => {
    const wrapper = mount(AppInput, {props:{modelValue:'existing-value',label:'Namespace',disabled:true}})
    const input=wrapper.get('input')
    expect(input.element.disabled).toBe(true)
    expect(input.element.value).toBe('existing-value')
    expect(input.element.closest('label')?.textContent).toContain('Namespace')
  })
  it('disables the actual select so a busy form cannot change scope', () => {
    const wrapper=mount(AppSelect,{props:{modelValue:'studio',label:'Publisher',disabled:true},slots:{default:'<option value="studio">Studio</option>'}})
    expect(wrapper.get('select').element.disabled).toBe(true)
    expect(wrapper.get('select').element.value).toBe('studio')
  })
})
