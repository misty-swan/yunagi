import { mount } from 'svelte';
import './shared/app.css';
import App from './App.svelte';
import { signalReady } from './shared/bench';

mount(App, { target: document.getElementById('app')! });
signalReady();
