import React from 'react';import ReactDOM from 'react-dom/client';import {BrowserRouter} from 'react-router-dom';import App from './App';import {AnalyzerProvider} from './hooks/useAnalyzer';import './index.css';
ReactDOM.createRoot(document.getElementById('root')!).render(<React.StrictMode><BrowserRouter><AnalyzerProvider><App/></AnalyzerProvider></BrowserRouter></React.StrictMode>);
