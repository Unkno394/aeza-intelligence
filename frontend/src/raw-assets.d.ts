declare module '*.csv?raw' {
 const content: string;
 export default content;
}

declare module '*.geojson' {
 const collection: {
  type: 'FeatureCollection';
  features: Array<{ id: string; geometry: unknown; properties?: Record<string, unknown> }>;
 };
 export default collection;
}
