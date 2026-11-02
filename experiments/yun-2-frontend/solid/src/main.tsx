import { render } from 'solid-js/web';
import './shared/app.css';
import App from './App';
import { signalReady } from './shared/bench';

render(() => <App />, document.getElementById('app')!);
signalReady();
